import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { connect } from "./api/connection";
import { App } from "./app/App";
import { PhonePage } from "./app/PhonePage";
import { Panel } from "./panel/Panel";
import "./styles.css";

const isPanel = location.pathname.startsWith("/panel");
document.documentElement.dataset.surface = isPanel ? "overlay" : "page";

const conn = connect(isPanel ? "overlay" : "browser");

const page = isPanel ? (
  <Panel conn={conn} />
) : location.pathname.startsWith("/phone") ? (
  <PhonePage conn={conn} />
) : (
  <App conn={conn} />
);

createRoot(document.getElementById("root")!).render(<StrictMode>{page}</StrictMode>);
