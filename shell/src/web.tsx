import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import WebApp from "./WebApp";
import "./styles/tokens.css";
import "./index.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <WebApp />
  </StrictMode>,
);
