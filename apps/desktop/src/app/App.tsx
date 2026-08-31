import { Navigate, Route, BrowserRouter, Routes } from 'react-router-dom';

import { AppShell } from './layouts/AppShell';
import { AppearanceSettingsPage, SettingsPage, SettingsProvider } from '../features/settings';
import { SftpView } from '../features/sftp';

/** 应用路由保留旧客户端的信息架构，功能按迁移阶段逐项接入。 */
export function App() {
  return (
    <SettingsProvider>
      <BrowserRouter>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/connections" element={null} />
            <Route path="/sftp" element={<SftpView />} />
            <Route path="/settings" element={<SettingsPage />}>
              <Route index element={<Navigate replace to="appearance" />} />
              <Route path="appearance" element={<AppearanceSettingsPage />} />
            </Route>
            <Route path="*" element={<Navigate replace to="/connections" />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </SettingsProvider>
  );
}
