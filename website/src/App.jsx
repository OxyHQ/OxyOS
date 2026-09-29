import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'
import Home from './pages/Home'
import Download from './pages/Download'

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="/download" element={<Download />} />
        <Route path="/download.html" element={<Navigate to="/download" replace />} />
      </Routes>
    </BrowserRouter>
  )
}
