// React entry point. Strict mode is disabled because Chart.js canvases do
// not tolerate the double-mount lifecycle during development without tearing
// down gracefully. The production build does not use strict mode anyway.
import { createRoot } from 'react-dom/client';

import { App } from './App';
import './index.css';

const container = document.getElementById('root');
if (!container) throw new Error('Root container #root not found');

createRoot(container).render(<App />);
