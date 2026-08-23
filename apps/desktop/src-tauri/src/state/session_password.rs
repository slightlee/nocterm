//! 会话级口令缓存：用户在 SSH 终端里交互输入的登录口令，仅在该连接尚有活跃会话期间驻留内存。
//!
//! 存在的原因：进程内 russh 必须在建连前拿到完整口令，无法像系统 OpenSSH 那样把
//! `password:` 提示交给远端 PTY 现场输入。若要求"必须先保存密码到系统凭据库才能连接"，
//! 密码登录就变成了硬性前置条件，与 PuTTY/Xshell/Termius 等主流客户端的行为不一致。
//!
//! 生命周期采用**租约计数**而非"整个应用运行期"：口令只在该连接还有活着的终端会话时保留，
//! 语义上等价于 OpenSSH `ControlMaster` 的连接复用，而不是"记住密码"。因此同一连接的
//! 第二个标签页与 SFTP 会话不会重复追问，但一旦全部会话关闭，口令立即丢弃，下次连接
//! 重新提示——未勾选保存的密码不应被静默留存，这是 `ssh`/PuTTY 的既有行为。
//!
//! 安全约定：
//! - 绝不写入 SQLite、系统凭据库、日志、测试快照或 IPC 事件，只在内存里按连接主键存放；
//! - 认证失败立即丢弃，避免错误口令在整个运行期反复重试并触发服务端封禁；
//! - 最后一个会话结束、删除连接与应用退出时清空；需要长期保存必须由用户显式走"保存密码"流程。

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// 一个连接的缓存条目：口令本身与仍在使用它的活跃会话数。
struct Entry {
    password: String,
    /// 活跃会话租约数；归零即代表该连接已无会话，口令随之丢弃。
    leases: usize,
}

/// 以连接主键为键的内存口令表。
///
/// 选用 `Mutex` 而非 `RwLock`：访问只发生在打开终端、结束会话与建立 SFTP 会话这类低频
/// 路径上，简单互斥足够，也省去读写锁在争用极低时反而更难推理的问题。
#[derive(Default)]
pub struct SessionPasswords {
    entries: Mutex<HashMap<i64, Entry>>,
}

impl SessionPasswords {
    /// 认证成功后登记口令并占用一个会话租约，两步在同一把锁内完成。
    ///
    /// 合并成一个方法是为了消除竞态：若先 `remember` 再单独计数，另一个会话的收尾
    /// 可能恰好在两步之间把条目清掉，导致刚建好的会话没有租约、口令被提前丢弃。
    /// 空串视为"没有口令"，直接移除而不是存入空条目，否则会挡住系统凭据库这条回退路径。
    pub fn retain(&self, connection_id: i64, password: &str) {
        let mut entries = self.guard();
        if password.is_empty() {
            entries.remove(&connection_id);
            return;
        }
        entries
            .entry(connection_id)
            .and_modify(|entry| {
                entry.password = password.to_string();
                entry.leases += 1;
            })
            .or_insert_with(|| Entry {
                password: password.to_string(),
                leases: 1,
            });
    }

    /// 归还一个会话租约；最后一个会话结束时丢弃口令，使下次连接重新提示输入。
    ///
    /// 条目可能已被 `forget`（认证失败、删除连接）提前清掉，因此缺失条目按无操作处理。
    pub fn release(&self, connection_id: i64) {
        let mut entries = self.guard();
        let Some(entry) = entries.get_mut(&connection_id) else {
            return;
        };
        entry.leases = entry.leases.saturating_sub(1);
        if entry.leases == 0 {
            entries.remove(&connection_id);
        }
    }

    /// 取出缓存口令的副本。返回所有权数据以便直接移动进后台连接线程。
    pub fn get(&self, connection_id: i64) -> Option<String> {
        self.guard()
            .get(&connection_id)
            .map(|entry| entry.password.clone())
    }

    /// 不论还有多少租约都丢弃某个连接的缓存口令：认证失败或删除连接时调用。
    pub fn forget(&self, connection_id: i64) {
        self.guard().remove(&connection_id);
    }

    /// 锁被污染时沿用受污染的数据而不是 panic：这张表没有需要保护的一致性不变量，
    /// 让所有连接因为一次无关线程的 panic 而彻底不可用，代价远大于收益。
    fn guard(&self) -> MutexGuard<'_, HashMap<i64, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for SessionPasswords {
    fn drop(&mut self) {
        // 进程退出前主动清空，缩短明文口令在内存中的驻留窗口。
        self.guard().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::SessionPasswords;

    #[test]
    fn keeps_the_password_per_connection_while_a_session_is_alive() {
        let passwords = SessionPasswords::default();
        passwords.retain(1, "first");
        passwords.retain(2, "second");
        assert_eq!(passwords.get(1).as_deref(), Some("first"));
        assert_eq!(passwords.get(2).as_deref(), Some("second"));

        passwords.release(1);
        assert_eq!(passwords.get(1), None);
        // 一个连接的会话收尾不能影响其他连接的缓存。
        assert_eq!(passwords.get(2).as_deref(), Some("second"));
    }

    #[test]
    fn drops_the_password_only_after_the_last_session_closes() {
        let passwords = SessionPasswords::default();
        // 同一连接开两个标签：第二次 retain 只增加租约，不改变可读到的口令。
        passwords.retain(5, "shared");
        passwords.retain(5, "shared");

        passwords.release(5);
        assert_eq!(passwords.get(5).as_deref(), Some("shared"));
        passwords.release(5);
        assert_eq!(passwords.get(5), None);
    }

    #[test]
    fn ignores_a_release_without_a_matching_entry() {
        let passwords = SessionPasswords::default();
        // 认证失败或删除连接会提前清掉条目，随后到来的会话收尾必须是无操作。
        passwords.retain(9, "kept");
        passwords.forget(9);
        passwords.release(9);
        passwords.release(9);
        assert_eq!(passwords.get(9), None);
    }

    #[test]
    fn forgets_regardless_of_outstanding_leases() {
        let passwords = SessionPasswords::default();
        passwords.retain(3, "kept");
        passwords.retain(3, "kept");
        passwords.forget(3);
        // 删除连接与认证失败必须立即清除，不能等租约归零。
        assert_eq!(passwords.get(3), None);
    }

    #[test]
    fn treats_an_empty_secret_as_no_secret() {
        let passwords = SessionPasswords::default();
        passwords.retain(7, "kept");
        passwords.retain(7, "");
        // 空串必须移除条目，否则会挡住系统凭据库这条正常回退路径。
        assert_eq!(passwords.get(7), None);
    }
}
