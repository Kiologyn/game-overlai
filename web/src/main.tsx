import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { connect } from "./api/connection";
import { App } from "./app/App";
import { PhonePage } from "./app/PhonePage";
import { SettingsPage } from "./app/SettingsPage";
import { Panel } from "./panel/Panel";
import { WidgetLayer } from "./widgets/WidgetLayer";
import "./styles.css";

const isPanel = location.pathname.startsWith("/panel");
const isWidgets = location.pathname.startsWith("/widgets");
// Both overlay windows are transparent: only the panel or the widgets are drawn.
document.documentElement.dataset.surface = isPanel || isWidgets ? "overlay" : "page";

const conn = connect(isPanel ? "overlay" : "browser");

const page = isPanel ? (
  <Panel conn={conn} />
) : isWidgets ? (
  <WidgetLayer conn={conn} />
) : location.pathname.startsWith("/phone") ? (
  <PhonePage conn={conn} />
) : location.pathname.startsWith("/settings") ? (
  <SettingsPage conn={conn} />
) : (
  <App conn={conn} />
);

createRoot(document.getElementById("root")!).render(<StrictMode>{page}</StrictMode>);
