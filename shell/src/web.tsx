import { StrictMode } from 'react';
import PlatformProvider from './PlatformProvider';
import { createRoot } from 'react-dom/client';
import WebApp from './WebApp';
import './styles/tokens.css';
import './index.css';

const rootElement = document.getElementById('root');
if (!rootElement) throw new Error('Missing #root element');

createRoot(rootElement).render(
  <StrictMode>
    <PlatformProvider>
      <WebApp />
    </PlatformProvider>
  </StrictMode>,
);
