import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { I18nProvider } from "./i18n";
import { RuntimeAgentsProvider } from "./runtimeAgents";
// @ts-ignore: side-effect CSS import type declarations are handled elsewhere
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider>
      <RuntimeAgentsProvider>
        <App />
      </RuntimeAgentsProvider>
    </I18nProvider>
  </React.StrictMode>
);
