# ito-pkg

A CLI tool for developing, packaging, and hosting plugins for the Ito application.

## Installation

```bash
cargo install --path .
```

## Usage

### 1. Create a New Plugin

Scaffold a new plugin project. Supported types are `manga`, `anime`, and `novel`.

```bash
ito-pkg new <plugin-name> --type <type>
# Or use the short flag
ito-pkg new <plugin-name> -t <type>
```

Example:
```bash
ito-pkg new my-manga-source -t manga
```

This creates a new directory with:
- `Cargo.toml`: Configured for `cdylib` compilation.
- `manifest.json`: Plugin metadata.
- `src/lib.rs`: Starter code implementing the provider trait.

### 2. Develop Your Plugin

Navigate to your plugin directory and implement the required trait methods in `src/lib.rs`.

Add an icon to your plugin by placing an `icon.png` file in the project root.

### 3. Package the Plugin

Compile your plugin to WebAssembly and bundle it into an `.ito` file.

```bash
ito-pkg pack
```

Output:
- `target/wasm32-unknown-unknown/release/<name>.wasm`: The compiled binary.
- `<name>.ito`: The final distributable package containing the Wasm binary, manifest, and icon.

`pack` reads the compiled module and records in the packaged manifest (not in your
`manifest.json`) what Ito needs to know before running it:

- `abi`: the ABI version the plugin was built for (its `ito_abi_version` export; 1 for plugins
  built before the export existed).
- `kinds`: the media kinds the plugin serves (`manga`, `anime`, `novel`), from its
  `ito_supported_kinds` export, which ABI 2 plugins must have. `type` stays your primary kind
  when it is one of them and becomes the first otherwise; older plugins serve their `type` alone.
- `imports`: every host function the plugin imports, as `module.name`.
- `sdk`: the ito-rs version from `Cargo.lock`.

It refuses a module that imports functions no Ito host provides, and an ABI 2 module that names
no known kind. The repository index carries the same fields, so Ito can refuse a plugin it
can't run before downloading it.

### 4. Verify a Package

Check the contents and validity of an `.ito` file: the manifest, that `main.wasm` imports only
functions Ito hosts provide, and that the manifest's `abi`, `kinds`, `type` and `imports` match
the module.

```bash
ito-pkg verify <path-to-ito-file>
```

### 5. Create a Local Repository

Organize multiple `.ito` files into a static repository structure that the Ito app can consume.

```bash
# Create a folder for your built plugins
mkdir plugins
mv *.ito plugins/

# Build the repository
ito-pkg repo --input plugins --output public --url "http://localhost:8080"
```

This generates:
- `public/index.json`: Repository index.
- `public/index.min.json`: Minified index.
- `public/packages/`: Directory containing `.ito` files.
- `public/icons/`: Directory containing extracted icons.

### 6. Serve the Repository

Host your repository locally for testing.

```bash
ito-pkg serve --path public --port 8080
```

Add `http://localhost:8080/index.json` to the Ito app to install your plugins.
