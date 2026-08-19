# Shared client E2E scenarios

These JSON files describe wallet actions, TON Connect dApp actions, UI
assertions, and screenshots. Web and iOS runners interpret the same scenario
steps with platform-specific drivers.

Edit the TypeScript definitions in `examples/web/e2e/scenarios/definitions.ts`.
Do not edit the JSON files directly. Regenerate them with:

```shell
bun --cwd examples/web e2e:export-scenarios
```

Each platform controls its selectors, process lifecycle, and screenshot
implementation. Both current runners record screenshots in dark appearance.
Git stores all screenshot PNG files in Git LFS.

The `localnet-activity` scenario starts an isolated Acton localnet. It checks
cached history, explicit refreshes, duplicate removal, ordering, and history
restoration after an application restart.
