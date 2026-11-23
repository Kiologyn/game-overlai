import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { connect } from "./api/connection";
import { App } from "./app/App";
import { Panel } from "./panel/Panel";
import "./styles.css";

const isPanel = location.pathname.startsWith("/panel");
document.documentElement.dataset.surface = isPanel ? "overlay" : "page";

const conn = connect(isPanel ? "overlay" : "browser");

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isPanel ? <Panel conn={conn} /> : <App conn={conn} />}</StrictMode>,
);
