# patchwork_editor

To build this, clone [nikitalita/godot @ patchwork-4.4](https://github.com/nikitalita/godot/tree/patchwork-4.4), then clone this repository into the `modules/patchwork_editor` directory.

```
git clone -b patchwork-4.4 https://github.com/nikitalita/godot
cd godot/modules
git clone https://github.com/nikitalita/patchwork_editor patchwork_editor --recurse-submodules

```

For rust plugin development:
install `watchexec` (e.g. `brew install watchexec`)
run `watchexec -e rs,toml cargo b` in the rust/plugin directory
Put the identity (e.g. Apple Development: Nikita Zatkovich (RFTZV7M2RV)) in the .cargo/.devidentity file to enable codesigning