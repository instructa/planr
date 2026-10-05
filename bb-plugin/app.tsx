// planr bb plugin frontend: the Planr board as a sidebar page and, compact, in a
// thread's side panel.
//
// Compiled by `bb plugin build` into dist/app.js + dist/app.css; React and
// @get-bb/plugin-sdk/app come from bb at load time. The page reads the board
// that server.ts gets from the engine and offers the three human actions;
// readiness, unlocks and validation stay in the engine.
import { definePluginApp } from "@get-bb/plugin-sdk/app";
import { PANEL_PATH } from "@/lib/route";
import { PlanrPage } from "./views/shell";
import { ThreadBoardPanel } from "./views/side-panel";

const ICON = "Workflow";

export default definePluginApp((app) => {
  app.slots.navPanel({
    id: "board",
    title: "Planr",
    icon: ICON,
    // Routed at /plugins/<pluginId>/board/<projectId>[/<view>[/<item>]].
    path: PANEL_PATH,
    component: PlanrPage,
  });
  // "Planr" in a thread's side-panel launcher: the board of that thread's project.
  app.slots.threadPanelAction({
    id: "board",
    title: "Planr",
    icon: ICON,
    layout: "flush",
    component: ThreadBoardPanel,
  });
});
