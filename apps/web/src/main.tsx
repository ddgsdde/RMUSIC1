import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import {isAndroidClient} from "./native";
import {mobileQuery} from "./MobileShell";
import {ErrorBoundary} from "./ErrorBoundary";
import "./styles.css";
import "./Mobile.css";
import "./MobileThemes.css";
import "./studio/Studio.css";
import "./MobileLayouts.css";
import "./studio/Record.css";
document.documentElement.toggleAttribute("data-native-android",isAndroidClient&&matchMedia(mobileQuery).matches);
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary><App /></ErrorBoundary>
  </React.StrictMode>,
);
