import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import WebApp from './WebApp';
import './styles/tokens.css';
import './index.css';

const rootElement = document.getElementById('root');
if (!rootElement) throw new Error('Missing #root element');

createRoot(rootElement).render(
  <StrictMode>
    <WebApp />
  </StrictMode>,
);
