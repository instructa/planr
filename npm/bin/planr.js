#!/usr/bin/env node
import { runNative } from "./native-launcher.js";

runNative({
  binaryName: "planr",
  overrideEnvironment: "PLANR_NATIVE_BIN",
  productName: "Planr",
});
