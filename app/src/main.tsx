import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource-variable/manrope/wght.css"; // Manrope, SIL Open Font License 1.1 (classic look)
import "@fontsource-variable/geist/wght.css"; // Geist, SIL Open Font License 1.1 (glass2, obsidian)
import "@fontsource/instrument-serif/400-italic.css"; // Instrument Serif, SIL OFL 1.1 (obsidian)
import "./styles/index.css";
import { inTauri } from "./bridge";
import { currentDesign } from "./design";

// In a plain browser (preview) the page needs a stand-in for the desktop behind the glass.
if (!inTauri) document.documentElement.classList.add("preview");
// Which look the bar uses (see design.ts); the styles key off this attribute.
document.documentElement.dataset.design = currentDesign();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
