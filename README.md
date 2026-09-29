# Tontoo Xcode

The App To Build, Run, Make Apps (AKA Xcode)

Start page: a 420x585 TontooUI card (~25% smaller than the Apple
reference) with an `xmark` close pill (no traffic lights), the centered
90px `Resources/icon.tico` (CoreIcon normal variant), `Open...` /
`New Project` buttons and a project list (`No Projects` while empty).
`New Project` opens a modal options sheet (EN/DE app name, version,
bundle ID, live bundle identifier preview, folder chooser from
`~/Documents`, `Cancel` / `Create`): `Create` scaffolds a real app
skeleton on disk and stores the project in CoreData, so it survives
restarts. Double-click (or finishing `Create`) hands over to a big
1100x700 example editor built on the `Sidebar` element (dead except
traffic lights). One persistent main process supervises both windows
(env handoff, no CLI). Run it with:

```bash
cargo run
```

Wiki: [wiki/MAIN.md](wiki/MAIN.md)

## Made for TontooOS

Explore more at https://github.com/TontooOS/Libs

## License

TCL v27.0