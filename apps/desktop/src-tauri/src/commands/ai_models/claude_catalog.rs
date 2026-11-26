//! Claude Code 具体版本目录需人工维护，不代表本机接口已提供这些模型。
//! 核对日期：2026-11-21；参考 pingdotgg/t3code 的 model-manifest.json，
//! 上游提交 fe9baf8cbb686b1665f42c518ef63df859e4cd24。
//! 维护提醒：发布新模型或升级 CLI 时，核对调用 ID、展示名、最低 CLI 版本、
//! current/legacy 状态、下线情况，并同步测试。上游未注明最低版本时不得猜测门槛。
//! 第三方接口是否支持具体 ID 由接口决定；本机别名及其映射始终优先保留。
//! 思考档位也需随模型核对，只维护原生 effort，不包含上游的提示词/编排扩展档位。
//! 模型默认档位参考 Claude Code 文档 https://code.claude.com/docs/en/model-config#adjust-effort-level。

use crate::dto::ai::{AiModelOption, AiModelSource, AiModelUnavailableReason};

// 每个固定 ID 的兼容性、历史状态和能力只在这一条记录维护，避免多个匹配表漂移。
// 档位集合仅复用已核对的原生值；新模型必须显式选择，不能按家族名称自动继承。
struct ModelSpec {
    id: &'static str,
    name: &'static str,
    minimum: Option<&'static str>,
    legacy: bool,
    efforts: &'static [&'static str],
    default_effort: Option<&'static str>,
}

const STANDARD: &[&str] = &["low", "medium", "high"];
const WITH_MAX: &[&str] = &["low", "medium", "high", "max"];
const EXTENDED: &[&str] = &["low", "medium", "high", "xhigh", "max"];

// 保留同代模型的展示优先级；固定版本不能改变本机动态别名的调用语义。
const VERSIONS: &[ModelSpec] = &[
    ModelSpec {
        id: "claude-opus-5-5",
        name: "Claude Opus 5.5",
        minimum: Some("2.1.280"),
        legacy: false,
        efforts: EXTENDED,
        default_effort: Some("medium"),
    },
    ModelSpec {
        id: "claude-sonnet-5-5",
        name: "Claude Sonnet 5.5",
        minimum: Some("2.1.284"),
        legacy: false,
        efforts: EXTENDED,
        default_effort: Some("medium"),
    },
    ModelSpec {
        id: "claude-haiku-5-5",
        name: "Claude Haiku 5.5",
        minimum: Some("2.1.293"),
        legacy: false,
        efforts: EXTENDED,
        default_effort: Some("medium"),
    },
    ModelSpec {
        id: "claude-fable-5-2",
        name: "Claude Fable 5.2",
        minimum: Some("2.1.321"),
        legacy: false,
        efforts: EXTENDED,
        default_effort: Some("medium"),
    },
    ModelSpec {
        id: "claude-fable-5-1",
        name: "Claude Fable 5.1",
        minimum: Some("2.1.257"),
        legacy: true,
        efforts: EXTENDED,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-opus-5",
        name: "Claude Opus 5",
        minimum: Some("2.1.219"),
        legacy: true,
        efforts: EXTENDED,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-sonnet-5",
        name: "Claude Sonnet 5",
        minimum: None,
        legacy: true,
        efforts: EXTENDED,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-fable-5",
        name: "Claude Fable 5",
        minimum: Some("2.1.169"),
        legacy: true,
        efforts: EXTENDED,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-opus-4-8",
        name: "Claude Opus 4.8",
        minimum: Some("2.1.154"),
        legacy: true,
        efforts: EXTENDED,
        default_effort: Some("high"),
    },
    // 文档默认 xhigh 不属于这里核对过的原生档位；输出时过滤，不映射为 max。
    ModelSpec {
        id: "claude-opus-4-7",
        name: "Claude Opus 4.7",
        minimum: Some("2.1.111"),
        legacy: true,
        efforts: WITH_MAX,
        default_effort: Some("xhigh"),
    },
    ModelSpec {
        id: "claude-opus-4-6",
        name: "Claude Opus 4.6",
        minimum: None,
        legacy: true,
        efforts: WITH_MAX,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-sonnet-4-6",
        name: "Claude Sonnet 4.6",
        minimum: None,
        legacy: true,
        efforts: STANDARD,
        default_effort: Some("high"),
    },
    ModelSpec {
        id: "claude-opus-4-5",
        name: "Claude Opus 4.5",
        minimum: None,
        legacy: true,
        efforts: WITH_MAX,
        default_effort: None,
    },
    ModelSpec {
        id: "claude-haiku-4-5",
        name: "Claude Haiku 4.5",
        minimum: None,
        legacy: true,
        efforts: &[],
        default_effort: None,
    },
];

/// 只接受稳定的三段版本，未知或预发布版本不冒充已验证兼容。
pub(super) fn parse_version(version: &str) -> Option<[u32; 3]> {
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}

pub(super) fn append_versions(
    models: &mut Vec<AiModelOption>,
    version: Option<&str>,
) -> Result<(), String> {
    let installed = version.and_then(parse_version);
    for spec in VERSIONS {
        let supports_fast_mode = fast_mode_minimum(spec.id).is_some_and(|required| {
            installed.is_some_and(|actual| Some(actual) >= parse_version(required))
        });
        if let Some(model) = models.iter_mut().find(|model| model.id == spec.id) {
            // CLI 报告项优先，但固定 ID 可以补充已核对且受版本约束的能力。
            model.supports_fast_mode = supports_fast_mode;
            continue;
        }
        let unavailable_reason = spec.minimum.and_then(|required| match installed {
            None => Some(AiModelUnavailableReason::CliVersionUnknown),
            Some(actual) if Some(actual) < parse_version(required) => {
                Some(AiModelUnavailableReason::CliUpdateRequired)
            }
            _ => None,
        });
        // 2.1.284 是已核对过 --effort 的 CLI 基线，不声称是该功能最早版本。
        let reasoning_efforts = if installed.is_some_and(|version| version >= [2, 1, 284]) {
            spec.efforts
        } else {
            &[]
        };
        // 只给可选的固定 ID 补默认值。CLI 别名可能映射到其他厂商的模型。
        let default_reasoning_effort = spec
            .default_effort
            .filter(|effort| unavailable_reason.is_none() && reasoning_efforts.contains(effort))
            .map(str::to_string);
        models.push(AiModelOption {
            id: spec.id.into(),
            name: spec.name.into(),
            source: AiModelSource::Catalog,
            is_legacy: spec.legacy,
            reasoning_efforts: reasoning_efforts.iter().map(|s| (*s).to_string()).collect(),
            default_reasoning_effort,
            supports_fast_mode,
            resolved_model: None,
            minimum_cli_version: spec.minimum.map(str::to_string),
            unavailable_reason,
        });
    }
    if models.len() > super::LIMIT {
        return Err(super::INVALID.into());
    }
    Ok(())
}

/// Headless fastMode 需要 Claude Code 2.1.205+；新型号还受各自首次支持版本约束。
fn fast_mode_minimum(id: &str) -> Option<&'static str> {
    match id {
        "claude-fable-5-2" => Some("2.1.321"),
        "claude-opus-5-5" => Some("2.1.280"),
        "claude-opus-5" => Some("2.1.219"),
        "claude-opus-4-8" => Some("2.1.205"),
        _ => None,
    }
}
