#!/usr/bin/env node
import { runNative } from "./native-launcher.js";

runNative({
  binaryName: "planr-browser-harness-adapter",
  overrideEnvironment: "PLANR_BROWSER_HARNESS_ADAPTER_BIN",
  productName: "Planr Browser Harness adapter",
});
