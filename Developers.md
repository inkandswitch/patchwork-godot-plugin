# Godot Collaboration Plugin: Developer's Guide

### Prerequisites

```bash
# 1. Git installed?
git --version

# 2. Rust installed?
rustc --version
cargo --version

# 3. Python 3 + SCons installed?
python3 --version
scons --version

# 4. C++ compiler available?
# Windows: Check Visual Studio is installed
# macOS: xcode-select -p
# Linux: gcc --version
```

If any are missing, see [Detailed Setup](#detailed-setup) below.

---

### Step-by-Step Build Process

#### Step 1: Clone Repositories

```bash
# Clone custom Godot fork
git clone -b patchwork-4.4 https://github.com/nikitalita/godot
cd godot

# Clone plugin into modules/
cd modules
git clone https://github.com/inkandswitch/patchwork-godot-plugin patchwork_editor --recurse-submodules

```

**Directory structure verification**:

```
godot/
├── modules/
│   ├── patchwork_editor/    ← Plugin here
│   │   ├── editor/          ← C++ module
│   │   ├── gdscript/        ← GDScript UI
│   │   ├── rust/plugin/     ← Rust core
│   │   ├── plugin.cfg
│   │   └── Patchwork.gdextension
│   └── ... (other modules)
├── bin/                     ← Compiled editor will be here
├── core/
└── SConstruct               ← Build configuration
```

---

### Step 2: Build

The `patchwork_build_tools/` directory provides powerful automation scripts that streamline the entire build, sync, and development workflow. This is the **recommended approach** for most developers. If you would like to build and develop manually, please continue to [Step 2b](#step-2b-manual-build) below.

### Step 2a: Build and Test Using Patchwork Build Tools (Recommended)

#### Quick Start

```bash
# Navigate to build tools
cd godot/patchwork_build_tools

# Windows (PowerShell - use .\ prefix)
.\pw.bat build                    # Build everything (Rust + Godot)
.\pw.bat sync --save              # Sync to your project and save path
.\pw.bat dev --hot-reload         # Start live development mode

# Linux/Mac
pw build
pw sync --save
pw dev --hot-reload
```

#### Step-by-Step: Using Build Scripts

**1. Build Godot with Patchwork**

```bash
# Full build (recommended for first time)
.\pw.bat build                # Windows
pw build                    # Linux/Mac

# Alternative: Build specific components
.\pw.bat build --rust         # Build only Rust plugin (Windows)
pw build --rust             # Build only Rust plugin (Linux/Mac)

.\pw.bat build --godot        # Build only Godot editor (Windows)
pw build --godot            # Build only Godot editor (Linux/Mac)

.\pw.bat build --clean        # Clean and rebuild everything (Windows)
pw build --clean            # Clean and rebuild everything (Linux/Mac)
```

**What happens:**

- Compiles Rust GDExtension plugin (`modules/patchwork_editor/rust/plugin/`)
- Copies compiled DLLs to the plugin directory
- Runs SCons to build Godot editor with Patchwork module
- Outputs editor to `godot/bin/godot.*.editor.*`

**Build times:**

- First build: 15-30 minutes
- Incremental builds: 1-5 minutes

**2. Sync Plugin to Your Project**

```bash
# Windows - First time (prompts for project path and saves it)
.\pw.bat sync --save

# Linux/Mac - First time
pw sync --save

# Subsequent syncs (uses saved path)
.\pw.bat sync                          # Windows
pw sync                              # Linux/Mac

# Advanced sync options
.\pw.bat sync --incremental            # Windows: Only sync changed files (5-10x faster)
pw sync --incremental                # Linux/Mac

.\pw.bat sync --hot-reload             # Windows: Sync while Godot is running
pw sync --hot-reload                 # Linux/Mac

.\pw.bat sync --diff                   # Windows: Preview what would sync
pw sync --diff                       # Linux/Mac
```

**What gets synced to `<your-project>/addons/patchwork/`:**

- `gdscript/` - All GDScript files (.gd, .tscn, .tres)
- `icons/` - UI icons (.svg, .png)
- `rust/plugin/` - Platform-specific DLLs
- `Patchwork.gdextension` - GDExtension configuration
- `plugin.cfg` - Plugin manifest

**Selective sync** (sync only specific files):

```bash
# Windows
.\pw.bat sync gdscript/sidebar.gd --hot-reload
.\pw.bat sync gdscript/ icons/

# Linux/Mac
pw sync gdscript/sidebar.gd --hot-reload
pw sync gdscript/ icons/
```

**3. Open Your Project with Custom Editor**

```bash
# Windows
godot\bin\godot.windows.editor.x86_64.exe -e --path "C:\path\to\your\project"

# Linux
godot/bin/godot.linuxbsd.editor.x86_64 -e --path "/path/to/your/project"

# macOS
godot/bin/godot.macos.editor.arm64 -e --path "/path/to/your/project"
```

**4. Enable the Plugin**

In Godot editor:

1. Go to **Project → Project Settings → Plugins**
2. Enable **Patchwork** plugin
3. You should see the Patchwork tab in the right sidebar

**5. Test Your Changes**

For rapid development iteration, use **Live Development Mode**:

```bash
# Windows - Start live dev mode (watches for changes and auto-syncs)
.\pw.bat dev --hot-reload

# Linux/Mac
pw dev --hot-reload

# Now edit any file:
# - GDScript changes appear in < 1 second!
# - Rust/C++ changes auto-rebuild and sync
```

#### Available Build Tools

**Main CLI:**

- `./pw` (Linux/Mac) or `pw.bat` (Windows) - Unified command interface

**Commands:**

```bash
# Windows
.\pw.bat build [--rust|--godot|--all|--clean]    # Build components
.\pw.bat sync [PATH] [--save|--hot-reload|--incremental]  # Sync to project
.\pw.bat watch [--hot-reload]                     # Watch files and auto-rebuild
.\pw.bat dev [--hot-reload|--no-build]            # Live development mode

# Linux/Mac
pw build [--rust|--godot|--all|--clean]      # Build components
pw sync [PATH] [--save|--hot-reload|--incremental]  # Sync to project
pw watch [--hot-reload]                       # Watch files and auto-rebuild
pw dev [--hot-reload|--no-build]              # Live development mode
```

**Core Scripts:**

- [build_patchwork.py](patchwork_build_tools/build_patchwork.py) - Builds Rust and Godot
- [sync_to_project.py](patchwork_build_tools/sync_to_project.py) - Syncs plugin files
- [watch_patchwork.py](patchwork_build_tools/watch_patchwork.py) - File watcher
- [dev_mode.py](patchwork_build_tools/dev_mode.py) - Live dev mode

#### Development Workflows

**For GDScript Development** (< 1 second feedback):

```bash
# Windows
.\pw.bat dev --no-build --hot-reload
# Linux/Mac
pw dev --no-build --hot-reload

# Edit files in modules/patchwork_editor/gdscript/
# Changes appear instantly in Godot!
```

**For Rust/C++ Development**:

```bash
# Windows
.\pw.bat dev --hot-reload
# Linux/Mac
pw dev --hot-reload

# Edit Rust/C++ files
# Auto-rebuilds and syncs on save
```

**Watch Mode** (auto-rebuild on changes):

```bash
# Windows
.\pw.bat watch --hot-reload
# Linux/Mac
pw watch --hot-reload

# Monitors all source files
# Rebuilds and syncs automatically
```

#### Hot Reload System

The hot reload feature lets you see changes without restarting Godot:

**How it works:**

1. Uses atomic file operations (no half-written files)
2. Triggers Godot reload via HTTP (port 6007) or file watching
3. Changes appear in < 1 second for GDScript

**Usage:**

```bash
# Windows
.\pw.bat sync --hot-reload
.\pw.bat sync --incremental --hot-reload
.\pw.bat dev --hot-reload

# Linux/Mac
pw sync --hot-reload
pw sync --incremental --hot-reload
pw dev --hot-reload
```

#### Configuration Files

- **`.patchwork_project`** - Stores saved project path (created by `--save`)
- **`.patchwork_manifest.json`** - MD5 hashes for incremental sync (auto-managed)

#### Troubleshooting Build Tools

**Missing dependencies:**

```bash
pip install scons watchdog psutil  # Install Python packages
```

**File locked errors:**

```bash
# Windows
.\pw.bat sync --hot-reload  # Use hot reload to sync while Godot is running

# Linux/Mac
pw sync --hot-reload
```

**For complete documentation:**

- [patchwork_build_tools/README.md](patchwork_build_tools/README.md) - Full build tools guide
- [patchwork_build_tools/HOT_RELOAD.md](patchwork_build_tools/HOT_RELOAD.md) - Hot reload details
- [patchwork_build_tools/SELECTIVE_SYNC.md](patchwork_build_tools/SELECTIVE_SYNC.md) - Selective sync
- [patchwork_build_tools/LIVE_DEV_MODE.md](patchwork_build_tools/LIVE_DEV_MODE.md) - Live dev mode

---

### Step 2b: Manual Build

If you prefer to build manually or need more control over the build process, follow these steps:

#### Prerequisites

If you haven't cloned the patchwork_editor module yet:

```bash
cd godot/modules
git clone https://github.com/nikitalita/patchwork_editor patchwork_editor --recurse-submodules
```

**1. Build the Rust Plugin**

```bash
# Navigate to patchwork_editor root directory
cd godot/modules/patchwork_editor

# Build in release mode (cargo workspace is in rust/plugin)
cargo build --release

# The DLL/library will be in: target/release/
# Copy it to the appropriate location for your platform:
```

**Platform-specific DLL locations:**

After building, copy the compiled library to the plugin directory:

**Windows (PowerShell):**

```powershell
# Create windows directory if it doesn't exist (run from patchwork_editor root)
New-Item -ItemType Directory -Force -Path rust\plugin\windows

# Copy DLL to plugin directory
Copy-Item target\release\patchwork_rust_core.dll `
  rust\plugin\windows\patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll
```

**Linux:**

```bash
# Create linux directory if it doesn't exist (run from patchwork_editor root)
mkdir -p rust/plugin/linux

# Copy .so to plugin directory
cp target/release/libpatchwork_rust_core.so \
   rust/plugin/linux/patchwork_rust_core.linux.x86_64-unknown-linux-gnu.so
```

**macOS (Intel):**

```bash
# macOS directory should already exist (run from patchwork_editor root)
# Copy .dylib to plugin directory
cp target/release/libpatchwork_rust_core.dylib \
   rust/plugin/macos/patchwork_rust_core.macos.x86_64-apple-darwin.dylib
```

**macOS (Apple Silicon):**

```bash
# macOS directory should already exist (run from patchwork_editor root)
# Copy .dylib to plugin directory
cp target/release/libpatchwork_rust_core.dylib \
   rust/plugin/macos/patchwork_rust_core.macos.arm64-apple-darwin.dylib
```

**2. Build Godot with Patchwork Module**

```bash
# Navigate to Godot root
cd godot

# Build Godot editor with SCons
# Basic build:
scons platform=windows target=editor

# With optimization and parallel jobs:
scons platform=windows target=editor -j8 production=yes

# Other platforms:
scons platform=linuxbsd target=editor -j8
scons platform=macos target=editor -j8
```

**Important SCons flags:**

- `platform=` - Target platform (windows, linuxbsd, macos)
- `target=editor` - Build the editor (not export templates)
- `-j8` - Use 8 parallel jobs (adjust to your CPU cores)
- `production=yes` - Optimized build (slower compile, faster runtime)
- `dev_build=yes` - Debug symbols (for development)

**Common Build Issues:**

If you encounter `LNK1106: invalid file or disk full` error on Windows:

```powershell
# 1. Clean build artifacts
scons --clean

# 2. Check disk space (need at least 20GB free)
Get-PSDrive C

# 3. Reduce parallel jobs (less memory/disk usage)
scons platform=windows target=editor -j4

# 4. Add antivirus exclusion for godot build directory
# Go to Windows Security → Virus & threat protection → Exclusions
# Add: C:\path\to\godot

# 5. Use shorter path if possible
# Move godot to C:\godot instead of deep nested folders
```

**3. Verify Build Output**

Check that the editor was built:

```bash
# Windows
ls bin/godot.windows.editor.x86_64.exe

# Linux
ls bin/godot.linuxbsd.editor.x86_64

# macOS
ls bin/godot.macos.editor.arm64
```

**4. Manually Copy Plugin to Your Project**

Create the plugin directory structure in your project:

```bash
# In your Godot project directory
mkdir -p addons/patchwork
```

Copy the necessary files from `godot/modules/patchwork_editor/` to `your-project/addons/patchwork/`:

```bash
# From godot/modules/patchwork_editor/ copy:

# GDScript files
cp -r gdscript/ YOUR_PROJECT/addons/patchwork/

# Icons
cp -r icons/ YOUR_PROJECT/addons/patchwork/

# Rust plugin DLLs (copy your platform's folder)
cp -r rust/plugin/windows/ YOUR_PROJECT/addons/patchwork/rust/plugin/windows/
# OR
cp -r rust/plugin/linux/ YOUR_PROJECT/addons/patchwork/rust/plugin/linux/
# OR
cp -r rust/plugin/macos/ YOUR_PROJECT/addons/patchwork/rust/plugin/macos/

# Configuration files
cp Patchwork.gdextension YOUR_PROJECT/addons/patchwork/
cp plugin.cfg YOUR_PROJECT/addons/patchwork/
```

**Example manual copy script (Windows PowerShell):**

```powershell
$SOURCE = "godot\modules\patchwork_editor"
$DEST = "C:\path\to\your\project\addons\patchwork"

# Create directory
New-Item -ItemType Directory -Force -Path $DEST

# Copy files
Copy-Item -Recurse -Force "$SOURCE\gdscript" "$DEST\gdscript"
Copy-Item -Recurse -Force "$SOURCE\icons" "$DEST\icons"
Copy-Item -Recurse -Force "$SOURCE\rust\plugin\windows" "$DEST\rust\plugin\windows"
Copy-Item -Force "$SOURCE\Patchwork.gdextension" "$DEST\"
Copy-Item -Force "$SOURCE\plugin.cfg" "$DEST\"
```

**Example manual copy script (Linux/Mac):**

```bash
SOURCE="godot/modules/patchwork_editor"
DEST="/path/to/your/project/addons/patchwork"

# Create directory
mkdir -p "$DEST"

# Copy files
cp -r "$SOURCE/gdscript" "$DEST/"
cp -r "$SOURCE/icons" "$DEST/"
cp -r "$SOURCE/rust/plugin/linux/" "$DEST/rust/plugin/linux/"  # or macos
cp "$SOURCE/Patchwork.gdextension" "$DEST/"
cp "$SOURCE/plugin.cfg" "$DEST/"
```

**5. Open Project with Custom Editor**

```bash
# Windows
godot\bin\godot.windows.editor.x86_64.exe -e --path "C:\path\to\your\project"

# Linux
godot/bin/godot.linuxbsd.editor.x86_64 -e --path "/path/to/your/project"

# macOS
godot/bin/godot.macos.editor.arm64 -e --path "/path/to/your/project"
```

**6. Understanding Patchwork's Architecture**

Patchwork is a **hybrid C++ module + GDExtension**, not a traditional plugin:

- **C++ Module** (`modules/patchwork_editor/`) - Built INTO your custom Godot editor
  - Automatically active when you launch the custom editor
  - Adds the Patchwork tab to the editor UI
  - Registers core classes (`PatchworkEditor`, `DiffInspector`, etc.)

- **GDExtension Component** (`addons/patchwork/`) - Provides Rust functionality
  - Contains the Rust plugin DLL/library
  - Contains GDScript UI components
  - Located in your project's `addons/patchwork/` folder

Because the C++ module is compiled directly into Godot (see [register_types.cpp:11-14](register_types.cpp#L11-L14)), Patchwork automatically initializes when the editor starts. The `plugin.cfg` file exists for compatibility but has an empty `script=""` field because there's no GDScript plugin script to enable/disable.

**In summary:** When you build Godot with Patchwork and copy the files to `addons/patchwork/`, the plugin is **always active** - you don't need to manually enable it in the Plugins menu. The Patchwork tab will appear automatically.

#### Manual Development Workflow

When developing manually, after making changes:

**For GDScript changes:**

```bash
# Linux/Mac - Copy only the changed file
cp godot/modules/patchwork_editor/gdscript/sidebar.gd \
   YOUR_PROJECT/addons/patchwork/gdscript/
```

```powershell
# Windows - Copy only the changed file
Copy-Item godot\modules\patchwork_editor\gdscript\sidebar.gd `
  YOUR_PROJECT\addons\patchwork\gdscript\

# Reload the scene in Godot (Scene → Reload Saved Scene)
```

**For Rust changes:**

```bash
# Linux/Mac - Rebuild Rust
cd godot/modules/patchwork_editor
cargo build --release

# Create directory if needed and copy .so/.dylib to plugin directory
mkdir -p rust/plugin/linux
cp target/release/libpatchwork_rust_core.so \
   rust/plugin/linux/patchwork_rust_core.linux.x86_64-unknown-linux-gnu.so
# OR for macOS (directory should already exist):
# cp target/release/libpatchwork_rust_core.dylib \
#    rust/plugin/macos/patchwork_rust_core.macos.arm64-apple-darwin.dylib

# Copy to project
cp rust/plugin/linux/*.so YOUR_PROJECT/addons/patchwork/rust/plugin/linux/
# OR for macOS:
# cp rust/plugin/macos/*.dylib YOUR_PROJECT/addons/patchwork/rust/plugin/macos/

# Restart Godot to reload library
```

```powershell
# Windows - Rebuild Rust
cd godot\modules\patchwork_editor
cargo build --release

# Create directory if needed
New-Item -ItemType Directory -Force -Path rust\plugin\windows

# Copy DLL to plugin directory
Copy-Item target\release\patchwork_rust_core.dll `
  rust\plugin\windows\patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll

# Copy to project
Copy-Item rust\plugin\windows\*.dll `
  YOUR_PROJECT\addons\patchwork\rust\plugin\windows\

# Restart Godot to reload DLL
```

**For C++ module changes:**

```bash
# Rebuild Godot
cd godot
scons platform=windows target=editor -j8

# Restart Godot with the new editor
bin/godot.windows.editor.x86_64.exe -e --path "YOUR_PROJECT"
```

#### Rust Plugin Development with Auto-Rebuild

For faster Rust development iteration, use `watchexec` to automatically rebuild on file changes:

**1. Install watchexec:**

```bash
# macOS
brew install watchexec

# Linux (Ubuntu/Debian)
sudo apt install watchexec

# Windows (via Cargo)
cargo install watchexec-cli

# Or download from: https://github.com/watchexec/watchexec/releases
```

**2. Run auto-rebuild from the patchwork_editor root:**

```bash
cd godot/modules/patchwork_editor

# Auto-rebuild on any .rs or .toml file change
watchexec -e rs,toml cargo b
```

This will:

- Watch for changes to `.rs` and `.toml` files
- Automatically run `cargo build` when changes are detected
- Show build output in the terminal

**3. macOS Code Signing (if needed):**

If you're on macOS and need code signing for the built library:

```bash
# In the rust/plugin directory, create the identity file
mkdir -p .cargo
echo "Apple Development: Your Name (TEAMID)" > .cargo/.devidentity

# Example:
echo "Apple Development: Nikita Zatkovich (RFTZV7M2RV)" > .cargo/.devidentity
```

**Tip:** Run `watchexec` in one terminal window and keep it running while you develop. Each time you save a Rust file, it will automatically rebuild!

#### When to Use Manual Build

**Use manual build when:**

- You need custom SCons flags or build configurations
- You're cross-compiling for different platforms
- You want to understand the build process in detail
- You're debugging build issues
- You prefer full control over each step

**Use build tools (Step 2a) when:**

- You want fast iteration (< 1 second for GDScript)
- You're actively developing the plugin
- You want automated workflows
- You prefer convenience over control

---

## Detailed Setup

### 1. Install Rust

```bash
# Install via rustup (recommended)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Restart terminal, then verify
rustc --version  # Should be 1.57.0 or higher
cargo --version
```

**Windows users**: Download from <https://rustup.rs/>

### 2. Install Python 3 + SCons

#### Windows

1. Download Python 3.10+ from <https://www.python.org/downloads/>
2. **Important**: Check "Add Python to PATH" during installation
3. Open new terminal:

   ```bash
   pip install scons
   ```

#### macOS

```bash
# Install Python 3 (usually pre-installed)
brew install python3

# Install SCons
pip3 install scons
```

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get update
sudo apt-get install python3 python3-pip
pip3 install scons
```

### 3. Install C++ Compiler

#### Windows

1. Download **Visual Studio 2019 or later**
2. During installation, select **"Desktop development with C++"**
3. Minimum components needed:
   - MSVC v142+ build tools
   - Windows 10 SDK

#### macOS

```bash
# Install Xcode Command Line Tools
xcode-select --install

# For this specific branch, Xcode 16+ is recommended
# Download from: https://developer.apple.com/xcode/
```

#### Linux (Ubuntu/Debian)

```bash
sudo apt-get install build-essential pkg-config libx11-dev libxcursor-dev \
    libxinerama-dev libgl1-mesa-dev libglu-dev libasound2-dev libpulse-dev \
    libudev-dev libxi-dev libxrandr-dev
```

### 4. Platform-Specific Setup

#### macOS: Vulkan SDK (Required for patchwork-4.4)

```bash
cd godot  # In the godot repository root
sh misc/scripts/install_vulkan_sdk_macos.sh
```

#### macOS: Code Signing

```bash
cd modules/patchwork_editor/rust/plugin

# Create identity file with your Apple Developer certificate
echo "Apple Development: Your Name (TEAMID)" > .cargo/.devidentity
```

Without this, macOS will show security warnings when loading the plugin.

---
