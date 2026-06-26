import { createRoot } from 'react-dom/client';
import { App } from './App';
import './index.css';

const root = document.getElementById('root');
if (!root) throw new Error('#root element not found');

// Note: no React.StrictMode — its dev-only double-mount would fire the Tauri
// status-listener subscription twice; the popover only ever renders one App.
createRoot(root).render(<App />);
