# Build the Rust plugin
cd rust/plugin
cargo build

# Change back to root directory and open Godot project
cd ../..
/Applications/Godot.app/Contents/MacOS/Godot moddable-pong/project.godot
