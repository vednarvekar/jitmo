import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource-variable/manrope/wght.css"; // Manrope, SIL Open Font License 1.1
import "./styles/index.css";
import { inTauri } from "./bridge";

// In a plain browser (preview) the page needs a stand-in for the desktop behind the glass.
if (!inTauri) document.documentElement.classList.add("preview");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
