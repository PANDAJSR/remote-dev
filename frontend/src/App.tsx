import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import DockviewApp from './Dockview';
import TerminalPage from './Terminal';
import './App.css';

function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<DockviewApp />} />
        <Route path="/terminal" element={<TerminalPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}

export default App;
