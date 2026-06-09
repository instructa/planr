# npm Development Package

The npm package name is `planr` and exposes a Node shim for local development and consumer E2E testing.

Do not use npm or npx as the primary public install path until the package includes platform-native Planr binaries. Normal users should install through the GitHub Release curl installer or manual GitHub Release downloads; Homebrew becomes the preferred package-manager path after the tap is published.

For local development:

```bash
cargo build --release
npm link
planr --version
```

For consumer E2E testing:

```bash
cd ~/projects/planr-test
npm link ../planr
npm run test:npm-planr
```

The npm shim looks for a native binary in this order:

1. `PLANR_NATIVE_BIN`;
2. `target/release/planr`;
3. `target/debug/planr`;
4. `dist/planr-1.0.0/planr`.

Release packages should include platform-native binaries or document the required `PLANR_NATIVE_BIN` override.
