import { Navigate, Route, BrowserRouter, Routes } from 'react-router-dom';

import { AppShell } from './layouts/AppShell';
import { SftpView } from '../features/sftp';

/** 应用路由保留旧客户端的信息架构，功能按迁移阶段逐项接入。 */
export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route path="/connections" element={null} />
          <Route path="/sftp" element={<SftpView />} />
          <Route path="/settings" element={null} />
          <Route path="*" element={<Navigate replace to="/connections" />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
