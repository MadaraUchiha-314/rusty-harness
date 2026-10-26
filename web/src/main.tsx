// oxlint-disable-next-line import/no-unassigned-import -- installs Buffer before the SDK loads
import "./polyfills";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app";
// oxlint-disable-next-line import/no-unassigned-import -- the stylesheet is a side effect
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
