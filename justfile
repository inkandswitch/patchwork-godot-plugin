build_dir := "build"
# The plugin directory in build_dir
plugin_dir := "patchwork"
# The directory for Rust binaries inside plugin_dir
plugin_bin_dir := "bin"
# The directory that all that public stuff goes into in the plugin itself
plugin_public_dir := "public"

moddable_platformer_repo := "git@github.com:endlessm/moddable-platformer.git"
moddable_platformer_checkout := "godot-4.6"

threadbare_repo := "git@github.com:endlessm/threadbare.git"
threadbare_checkout := "godot-4.6"

godot_repo := "git@github.com:godotengine/godot.git"
godot_checkout := "bb92a4c8e27e30cdec05ab6d540d724b9b3cfb72"

godot_formatters_repo := "git@github.com:nikitalita/GodotFormatters.git"
godot_formatters_checkout := "master"

# The directory of the editor module that needs to be compiled with Godot
editor_dir := "editor"
# The directory of our source "public" folder that includes gdscript & assets that should be linked directly into the plugin
public_dir := "public"

default_server := "24.199.97.236:8080"

# TODO: expose this as an environment variable
# Get the default architecture
default_arch := shell("rustc --version --verbose | grep host | awk '{print $2}'")

default:
  just --list

# Safely symlink src to dest
_symlink src dest:
    #!/usr/bin/env python3
    # Use python for a cross-platform safe symlink that isn't dependent on Git bash installation settings
    from pathlib import Path
    src = Path("{{src}}")
    dest = Path("{{dest}}")
    if dest.exists():
        if not dest.is_symlink():
            print(f"Destination {{dest}} already exists and is NOT a symlink!")
            exit(1)

        target = (dest.parent / dest.readlink()).resolve()
        
        if not target.samefile(src.resolve()):
            print(f"Destination {{dest}} already exists, but it points to {target} instead of {src.resolve()}.")
            exit(1)

        # symlink already exists and is valid
        exit(0)
    
    try:
        dest.symlink_to(src.resolve(), True)
        exit(0)
    except OSError as e:
        print((f"Failed to create symlink from {dest.resolve()} to {src.resolve()}. "
            "If you are on Windows, you MUST enable Developer Mode in system settings."))
        exit(1)


# Create build/
@_make-build-dir:
    mkdir -p {{build_dir}}

# Create build/plugin
@_make-plugin-dir: _make-build-dir
    mkdir -p {{build_dir}}/{{plugin_dir}}

# Clone a repository to a directory and check out a commit.
_clone repo_url directory checkout:
    #!/usr/bin/env sh
    set -euxo pipefail
    if [[ ! -d "{{directory}}" ]]; then
        git clone "{{repo_url}}" "{{directory}}"
    else
        if git -C "{{directory}}" remote | grep -q '^origin$'; then
            git -C "{{directory}}" remote set-url origin "{{repo_url}}"
        else
            git -C "{{directory}}" remote add origin "{{repo_url}}"
        fi
    fi

    # try checkout, if we get an error fetch then try again.
    # this won't pull updates from the remote, but that's probably fine for now.
    # if we need to handle local changes, this will need to be refactored (to force reset?)
    if git -C "{{directory}}"  checkout "{{checkout}}" | grep -q '^fatal'; then
        git -C "{{directory}}" fetch --all
        git -C "{{directory}}"  checkout "{{checkout}}"
    fi


# Clone our desired project and checkout the proper commit.
[arg('project', pattern='moddable-platformer|threadbare')]
_acquire-project project: _make-build-dir
    #!/usr/bin/env sh
    set -euxo pipefail
    if [[ "{{project}}" = "moddable-platformer" ]]; then
        just _clone "{{moddable_platformer_repo}}" "{{build_dir}}/{{project}}" "{{moddable_platformer_checkout}}"
    else
        just _clone "{{threadbare_repo}}" "{{build_dir}}/{{project}}" "{{threadbare_checkout}}"
    fi

# Clone the Godot repository and checkout the proper commit.
_acquire-godot: _make-build-dir
    just _clone "{{godot_repo}}" "{{build_dir}}/godot" "{{godot_checkout}}"

# Clone the GodotFormatters repository and checkout the proper commit.
_acquire-formatters: _make-build-dir
    just _clone "{{godot_formatters_repo}}" "{{build_dir}}/GodotFormatters" "{{godot_formatters_checkout}}"

# Link our plugin build directory to the desired project.
_link-project project: (_acquire-project project) _make-plugin-dir
    mkdir -p "{{build_dir}}/{{project}}/addons"
    just _symlink "{{build_dir}}/{{plugin_dir}}" "{{build_dir}}/{{project}}/addons/patchwork"

# Link our custom Godot editor module
_link-godot: _acquire-godot
    mkdir -p "{{build_dir}}/godot/"
    just _symlink "{{editor_dir}}" "{{build_dir}}/godot/modules/patchwork_editor"

# Link the assets directory for our plugin
_link-public: _make-plugin-dir
    just _symlink "{{public_dir}}" "{{build_dir}}/{{plugin_dir}}/{{plugin_public_dir}}"

# Build the Godot editor with our editor module linked in. Available profiles are release, debug, or sani (for use_asan=yes)
[arg('profile', pattern='release|debug|sani')]
build-godot profile: _link-godot
    #!/usr/bin/env sh
    set -euxo pipefail
    cd "{{build_dir}}/godot"
    # TODO: figure out a way to see if scons actually needs a run, since this takes forever even when built
    if [[ {{profile}} = "release" ]] ; then
        scons dev_build=no debug_symbols=no target=editor deprecated=yes minizip=yes compiledb=yes
    elif [[ {{profile}} = "sani" ]] ; then
        scons dev_build=yes target=editor compiledb=yes deprecated=yes minizip=yes tests=yes use_asan=yes 
    else
        scons dev_build=yes target=editor compiledb=yes deprecated=yes minizip=yes tests=yes 
    fi

# Build the Rust plugin binaries.
_build-plugin architecture profile:
    cargo build --profile="{{profile}}" --target="{{architecture}}"

# Build the multi-arch target for MacOS.
[parallel]
_build-plugin-all-macos profile: (_build-plugin "aarch64-apple-darwin" profile) (_build-plugin "x86_64-apple-darwin" profile) _make-plugin-dir
    # Rather than copying the generated .dylibs, we combine them into a single one.
    lipo -create -output {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.dylib \
        target/aarch64-apple-darwin/{{profile}}/libpatchwork_rust_core.dylib \
        target/x86_64-apple-darwin/{{profile}}/libpatchwork_rust_core.dylib
    # TODO: Perhaps sign here instead of in github actions?

[parallel]
_build-plugin-single-arch architecture profile: (_build-plugin architecture profile) _make-plugin-dir
    #!/usr/bin/env sh
    set -euo pipefail
    mkdir -p {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}

    # Copy the entire macos directory to get the Resources framework directory
    rm -rf "{{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/libpatchwork_rust_core.macos.framework"
    cp -r "rust/macos/libpatchwork_rust_core.macos.framework" "{{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}"

    if [ -f "target/{{architecture}}/{{profile}}/patchwork_rust_core.dll" ] ; then
        cp "target/{{architecture}}/{{profile}}/patchwork_rust_core.dll" \
            {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/patchwork_rust_core.windows.{{architecture}}.dll
    fi

    if [ -f "target/{{architecture}}/{{profile}}/patchwork_rust_core.so" ] ; then
        cp "target/{{architecture}}/{{profile}}/patchwork_rust_core.so" \
            {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/patchwork_rust_core.linux.{{architecture}}.so
    fi

    if [ -f "target/{{architecture}}/{{profile}}/patchwork_rust_core.dylib" ] ; then
        cp "target/{{architecture}}/{{profile}}/patchwork_rust_core.dylib" \
            {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.macos.dylib
    fi
    
    if [ -f "target/{{architecture}}/{{profile}}/patchwork_rust_core.pdb" ] ; then
        cp "target/{{architecture}}/{{profile}}/patchwork_rust_core.pdb" \
            {{build_dir}}/{{plugin_dir}}/{{plugin_bin_dir}}/patchwork_rust_core.pdb
    fi

# Write plugin.cfg and Patchwork.gdextension
_configure-patchwork: _make-plugin-dir
    #!/usr/bin/env python3
    import os
    import subprocess

    # load the version from git
    git_describe = subprocess.check_output(["git", "describe", "--tags", "--abbrev=6"]).decode("utf-8").strip()

    # if it has more than two `-` in the version, replace all the subsequent `-` with `+`
    if git_describe.count("-") >= 2:
        first_index = git_describe.find("-")
        if first_index != -1:
            git_describe = git_describe[:first_index] + "-" + git_describe[first_index + 1 :].replace("-", "+")

    print(f"Loaded version from Git repository: {git_describe}")

    with open("{{build_dir}}/{{plugin_dir}}/plugin.cfg", "a") as file:
        file.write(f"""[plugin]
    name="Patchwork"
    description="Version control for Godot"
    author="Ink & Switch"
    version="{git_describe}"
    script=""
    """)
    
    with open("{{build_dir}}/{{plugin_dir}}/Patchwork.gdextension", "a") as file:
        file.write(f"""[configuration]
    entry_symbol = "gdext_rust_init"
    compatibility_minimum = 4.6
    reloadable = true

    [libraries]
    linux.debug.x86_64 =        "bin/libpatchwork_rust_core.linux.x86_64-unknown-linux-gnu.so"
    linux.release.x86_64 =      "bin/libpatchwork_rust_core.linux.x86_64-unknown-linux-gnu.so"
    linux.debug.arm64 =         "bin/libpatchwork_rust_core.linux.aarch64-unknown-linux-gnu.so"
    linux.release.arm64 =       "bin/libpatchwork_rust_core.linux.aarch64-unknown-linux-gnu.so"
    linux.debug.arm32 =         "bin/libpatchwork_rust_core.linux.armv7-unknown-linux-gnueabihf.so"
    linux.release.arm32 =       "bin/libpatchwork_rust_core.linux.armv7-unknown-linux-gnueabihf.so"
    windows.debug.x86_64 =      "bin/patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll"
    windows.release.x86_64 =    "bin/patchwork_rust_core.windows.x86_64-pc-windows-msvc.dll"
    windows.debug.arm64 =       "bin/patchwork_rust_core.windows.aarch64-pc-windows-msvc.dll"
    windows.release.arm64 =     "bin/patchwork_rust_core.windows.aarch64-pc-windows-msvc.dll"
    macos.debug =               "bin/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.macos.dylib"
    macos.release =             "bin/libpatchwork_rust_core.macos.framework/libpatchwork_rust_core.macos.dylib"
    """)

# Build the plugin and output it to the plugin build dir. For MacOS multi-arch, use architecture=all-apple-darwin to build all architectures.
[parallel]
[arg('profile', pattern='release|debug')]
build-patchwork profile architecture=(default_arch): _configure-patchwork _link-public
    #!/usr/bin/env sh
    set -euxo pipefail
    if [[ "{{architecture}}" = "all-apple-darwin" ]] ; then
        just _build-plugin-all-macos "{{profile}}"
        exit 0
    fi

    profile="release"
    if [[ "{{profile}}" = "debug" ]] ; then
        profile="release_debug"
    fi
    
    just _build-plugin-single-arch "{{architecture}}" "$profile"

# Reset the Godot repository, removing the linked module and resetting the repo state.
clean-godot:
    #!/usr/bin/env sh
    set -euxo pipefail
    if [[ ! -d "{{build_dir}}" ]]; then
        exit 0
    fi
    cd "{{build_dir}}"
    
    set -euxo pipefail
    if [[ ! -d "godot" ]]; then
        exit 0
    fi
    cd godot
    git checkout -f {{godot_checkout}}
    git clean -xdf

# Remove any built Patchwork artifacts.
clean-patchwork:
    #!/usr/bin/env sh
    set -euxo pipefail
    cargo clean
    if [[ ! -d "{{build_dir}}/{{plugin_dir}}" ]]; then
        exit 0
    fi
    rm -rf "{{build_dir}}/{{plugin_dir}}"

# Clean a single project, resetting the repository and unlinking Patchwork.
[arg('project', pattern='moddable-platformer|threadbare')]
clean-project project:
    #!/usr/bin/env sh
    set -euxo pipefail
    if [[ "{{project}}" = "moddable-platformer" ]]; then
        checkout="{{moddable_platformer_checkout}}"
    else
        checkout="{{threadbare_checkout}}"
    fi
    
    if [[ ! -d "{{build_dir}}" ]]; then
        exit 0
    fi
    cd "{{build_dir}}"
    
    if [[ ! -d "{{project}}" ]]; then
        exit 0
    fi
    cd {{project}}
    git checkout -f "$checkout"
    git clean -xdf

# Clean Patchwork, and the projects threadbare, moddable-platformer.
clean: (clean-project "threadbare") (clean-project "moddable-platformer") clean-patchwork

# Write to the project .cfg with a new server url
[arg('project', pattern='moddable-platformer|threadbare')]
_write-url project url: (_link-project project)
    #!/usr/bin/env python3
    import os
    import subprocess
    from pathlib import Path

    path = "{{build_dir}}/{{project}}/patchwork.cfg"

    try:
        f = open(path)
    except FileNotFoundError:
        lines = []
    else:
        with f: lines = f.readlines()

    new_lines: list[str] = []
    found_patchwork = False
    for line in lines:
        # place the server url immediately after patchwork
        if line.startswith("[patchwork]"):
            found_patchwork = True
            new_lines.append(line)
            new_lines.append('server_url="{{url}}"\n')
        # skip future server URLs
        elif not line.startswith("server_url="):
            new_lines.append(line)
    
    if not found_patchwork:
        new_lines = ['[patchwork]\n', 'server_url="{{url}}"']

    with open(path, "w") as file:
        file.writelines(new_lines)

# Prepare a project for launch with Godot. Available projects are threadbare, moddable-platformer.
[parallel]
[arg('project', pattern='moddable-platformer|threadbare')]
[arg('patchwork_profile', pattern='release|debug')]
[arg('godot_profile', pattern='release|debug|sani')]
prepare project="moddable-platformer" patchwork_profile="release" godot_profile="release" server_url=default_server:\
        (_link-project project) (build-godot godot_profile) (build-patchwork patchwork_profile)


# Launch a project with Godot. Available projects are threadbare, moddable-platformer.
[arg('project', pattern='moddable-platformer|threadbare')]
[arg('patchwork_profile', pattern='release|debug')]
[arg('godot_profile', pattern='release|debug|sani')]
launch project="moddable-platformer" patchwork_profile="release" godot_profile="release" server_url=default_server: \
        (prepare project patchwork_profile godot_profile server_url)
    #!/usr/bin/env sh
    set -euxo pipefail
    
    case "{{arch()}}" in
        "x86_64")
            arch=x86_64 ;;
        "aarch64")
            arch=arm64 ;;
        *)
            echo "Unsupported architecture for development: {{arch()}}."
            echo "If you think this architecture should be supported, please open an issue on Github with your use-case and system details."
            exit 1 ;;
    esac
    
    case "{{os()}}" in
        "windows")
            ext=".exe" ;;
        "linux")
            ext="" ;;
        "mac")
            ext="" ;;
        *)
            echo "Unsupported OS for development: {{os()}}."
            echo "If you think this OS should be supported, please open an issue on Github with your use-case and system details."
            exit 1 ;;
    esac
    
    if [[ {{godot_profile}} = "release" ]] ; then
        godot_path="{{build_dir}}/godot/bin/godot.{{os()}}.editor.$arch$ext"
    else
        godot_path="{{build_dir}}/godot/bin/godot.{{os()}}.editor.dev.$arch$ext"
    fi

    $godot_path -e --path "{{build_dir}}/{{project}}"