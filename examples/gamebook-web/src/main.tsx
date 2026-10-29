import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing root element");
if (new URLSearchParams(location.search).has("online")) {
  document.title = "Narrata online story journey";
  void import("./online").then(module => module.mountOnlineReference(root));
} else createRoot(root).render(<StrictMode><App /></StrictMode>);
