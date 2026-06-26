import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ScreenshotOverlayWindow from "./components/ScreenshotOverlayWindow";
import { I18nProvider } from "./i18n";
import { RuntimeAgentsProvider } from "./runtimeAgents";
// @ts-ignore: side-effect CSS import type declarations are handled elsewhere
import "./styles.css";

const isScreenshotOverlay = new URLSearchParams(window.location.search).has("screenshotOverlay");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider>
      {isScreenshotOverlay ? (
        <ScreenshotOverlayWindow />
      ) : (
        <RuntimeAgentsProvider>
          <App />
        </RuntimeAgentsProvider>
      )}
    </I18nProvider>
  </React.StrictMode>
);
