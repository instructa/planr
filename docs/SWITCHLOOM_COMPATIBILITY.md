# Archived Switchloom v0.3.2 Compatibility Evidence

This document records a historical compatibility check performed for the external `switchloom@0.3.2` package. It is not a current Planr release gate or a promise about future Switchloom artifacts, routes, role names, host telemetry, or lifecycle behavior.

The historical check established only that the pinned v0.3.2 package could generate repository-local `.planr/agents.toml` and `.planr/policy.toml` declarations that the Planr version at that time consumed successfully. The accompanying one-off cross-product oracle has been removed from Planr.

Planr's stable boundary remains:

- Planr works without Switchloom or routing declarations.
- Planr treats profiles, routes, model names, efforts, and fallbacks as provider-neutral repository data.
- Requested routing metadata is not effective execution proof.
- External tools own their install, compilation, generated host files, reload requirements, and uninstall lifecycle.

For current Switchloom behavior, consult the external project and its versioned documentation. Any future compatibility claim requires a new bounded check owned by that integration; Planr does not retain a permanent package-specific oracle.
