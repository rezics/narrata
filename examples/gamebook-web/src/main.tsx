import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing root element");
if (new URLSearchParams(location.search).has("reference")) {
  document.title = "Narrata cold reference journey";
  void import("./reference").then(module => module.mountColdReference(root));
} else if (new URLSearchParams(location.search).has("online")) {
  document.title = "Narrata online story journey";
  void import("./online").then(module => module.mountOnlineReference(root));
} else void import("./App").then(({ App }) => createRoot(root).render(<StrictMode><App /></StrictMode>));
