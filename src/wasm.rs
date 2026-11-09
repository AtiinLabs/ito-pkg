//! What a plugin's WebAssembly module needs from the host, and the ABI it was built for.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use wasmparser::{
    CompositeInnerType, ExternalKind, FuncType, FunctionBody, Operator, Parser, Payload, TypeRef,
    ValType,
};

/// The host functions an Ito host provides, by module. Keep in step with ito-runner's import
/// table (`WasmBridge.buildImportTable`): a plugin importing anything else can't load.
pub const HOST_IMPORTS: &[(&str, &[&str])] = &[
    ("ito:core/net", &["fetch", "fetch_v2", "fetch_read"]),
    (
        "ito:core/html",
        &[
            "parse",
            "select",
            "text",
            "attr",
            "free",
            "own_text",
            "html_content",
            "outer_html",
        ],
    ),
    ("ito:core/std", &["print"]),
    ("ito:core/defaults", &["set", "get", "remove", "set_secret"]),
    ("ito:core/env", &["get_languages"]),
    (
        "ito:core/webview",
        &[
            "webview_load_url",
            "webview_execute_js",
            "webview_read_result",
        ],
    ),
];

/// The export the SDK adds with the plugin's ABI version.
const ABI_EXPORT: &str = "ito_abi_version";

/// The export the SDK adds, from ABI 2, with the kinds the plugin serves as a bitmask.
const KINDS_EXPORT: &str = "ito_supported_kinds";

/// Media kinds by bit in `ito_supported_kinds`, as manifests name them.
pub const KINDS: [&str; 3] = ["manga", "anime", "novel"];

/// The ABI a plugin built before `ito_abi_version` existed was built for.
pub const UNVERSIONED_ABI: u32 = 1;

pub struct WasmInterface {
    /// Every import, as (module, name).
    pub imports: Vec<(String, String)>,
    /// The constant `ito_abi_version` returns, if the module exports it.
    pub abi_export: Option<u32>,
    /// The constant `ito_supported_kinds` returns, if the module exports it.
    pub kinds_export: Option<u32>,
    /// Whether the module has a start section, which runs code at instantiation, before the
    /// host has called any export.
    pub has_start_section: bool,
}

impl WasmInterface {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut imports = Vec::new();
        let mut has_start_section = false;
        // Function types by type index, and the type index of every function: imports first, then
        // the module's own, which is the order of the function index space.
        let mut types: Vec<Option<FuncType>> = Vec::new();
        let mut function_types: Vec<u32> = Vec::new();
        let mut imported_functions = 0u32;
        let mut defined_functions = 0u32;
        let mut abi_function = None;
        let mut abi_export = None;
        let mut kinds_function = None;
        let mut kinds_export = None;

        for payload in Parser::new(0).parse_all(bytes) {
            match payload.context("Invalid WebAssembly module")? {
                Payload::TypeSection(reader) => {
                    for group in reader {
                        for ty in group.context("Invalid type")?.into_types() {
                            types.push(match ty.composite_type.inner {
                                CompositeInnerType::Func(func) => Some(func),
                                _ => None,
                            });
                        }
                    }
                }
                Payload::ImportSection(reader) => {
                    for import in reader.into_imports() {
                        let import = import.context("Invalid import")?;
                        if let TypeRef::Func(ty) | TypeRef::FuncExact(ty) = import.ty {
                            imported_functions += 1;
                            function_types.push(ty);
                        }
                        imports.push((import.module.to_string(), import.name.to_string()));
                    }
                }
                Payload::FunctionSection(reader) => {
                    for ty in reader {
                        function_types.push(ty.context("Invalid function")?);
                    }
                }
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.context("Invalid export")?;
                        if export.kind == ExternalKind::Func {
                            let (name, slot) = match export.name {
                                ABI_EXPORT => (ABI_EXPORT, &mut abi_function),
                                KINDS_EXPORT => (KINDS_EXPORT, &mut kinds_function),
                                _ => continue,
                            };
                            require_nullary_i32(name, export.index, &function_types, &types)?;
                            *slot = Some(export.index);
                        }
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    let index = Some(imported_functions + defined_functions);
                    if abi_function == index {
                        abi_export = Some(constant_result(&body, ABI_EXPORT)?);
                    }
                    if kinds_function == index {
                        kinds_export = Some(constant_result(&body, KINDS_EXPORT)?);
                    }
                    defined_functions += 1;
                }
                Payload::StartSection { .. } => has_start_section = true,
                _ => {}
            }
        }

        if abi_function.is_some() && abi_export.is_none() {
            bail!("{ABI_EXPORT} must be defined in the module, not imported");
        }
        if kinds_function.is_some() && kinds_export.is_none() {
            bail!("{KINDS_EXPORT} must be defined in the module, not imported");
        }
        Ok(Self {
            imports,
            abi_export,
            kinds_export,
            has_start_section,
        })
    }

    /// The ABI the plugin was built for: its `ito_abi_version`, or 1 without one.
    pub fn abi(&self) -> u32 {
        self.abi_export.unwrap_or(UNVERSIONED_ABI)
    }

    /// Fails unless the module is built for the ABI the caller expects. `None` accepts any.
    fn require_abi(&self, expected: Option<u32>) -> Result<()> {
        match expected {
            Some(expected) if expected != self.abi() => bail!(
                "The plugin is built for ABI {}, but ABI {expected} is expected",
                self.abi()
            ),
            _ => Ok(()),
        }
    }

    /// Fails when the module has a start section. Hosts instantiate a plugin before calling any
    /// of its exports, so start-section code would run outside the host's calls.
    fn require_no_start_section(&self) -> Result<()> {
        if self.has_start_section {
            bail!("The plugin has a start section; modules must do their work only in exports");
        }
        Ok(())
    }

    /// Reads a module and refuses what no Ito host can run: an invalid module, a start section,
    /// imports no host provides, or an ABI other than `expected_abi` when one is required. `pack`
    /// and `verify` both go through this, so they can't disagree about what is acceptable.
    pub fn inspect(bytes: &[u8], expected_abi: Option<u32>) -> Result<Self> {
        let interface = Self::read(bytes)?;
        interface.require_no_start_section()?;
        let unknown_imports = interface.unknown_imports();
        if !unknown_imports.is_empty() {
            bail!(
                "The plugin imports functions no Ito host provides: {}",
                unknown_imports.join(", ")
            );
        }
        interface.require_abi(expected_abi)?;
        Ok(interface)
    }

    /// The kinds the plugin serves, in `KINDS` order: from `ito_supported_kinds`, which ABI 2
    /// plugins must export. None for older plugins, which serve their manifest `type`; hosts
    /// ignore the export below ABI 2, so it is ignored here too.
    pub fn kinds(&self) -> Result<Option<Vec<&'static str>>> {
        if self.abi() < 2 {
            return Ok(None);
        }
        let Some(mask) = self.kinds_export else {
            bail!("An ABI {} plugin must export {KINDS_EXPORT}", self.abi());
        };
        let known = (1u32 << KINDS.len()) - 1;
        if mask == 0 || mask & !known != 0 {
            bail!("{KINDS_EXPORT} must name at least one of {KINDS:?}, got {mask:#b}");
        }
        Ok(Some(
            KINDS
                .iter()
                .enumerate()
                .filter(|(bit, _)| mask & 1 << bit != 0)
                .map(|(_, kind)| *kind)
                .collect(),
        ))
    }

    /// Imports no Ito host provides, as "module.name", sorted.
    pub fn unknown_imports(&self) -> Vec<String> {
        let unknown: BTreeSet<String> = self
            .imports
            .iter()
            .filter(|(module, name)| {
                !HOST_IMPORTS.iter().any(|(host_module, names)| {
                    host_module == module && names.contains(&name.as_str())
                })
            })
            .map(|(module, name)| format!("{module}.{name}"))
            .collect();
        unknown.into_iter().collect()
    }

    /// Every import, as "module.name", sorted and without duplicates. Hosts compare this with the
    /// functions they provide before downloading the plugin.
    pub fn import_names(&self) -> Vec<String> {
        let names: BTreeSet<String> = self
            .imports
            .iter()
            .map(|(module, name)| format!("{module}.{name}"))
            .collect();
        names.into_iter().collect()
    }
}

/// Fails unless the function at `index` is `() -> i32`, the type hosts call `export` through.
fn require_nullary_i32(
    export: &str,
    index: u32,
    function_types: &[u32],
    types: &[Option<FuncType>],
) -> Result<()> {
    let signature = function_types
        .get(index as usize)
        .and_then(|ty| types.get(*ty as usize))
        .and_then(Option::as_ref)
        .with_context(|| format!("{export} refers to a function with no known type"))?;
    if signature.params().is_empty() && signature.results() == [ValType::I32] {
        return Ok(());
    }
    let list = |types: &[ValType]| {
        types
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    bail!(
        "{export} must be a function () -> i32, but it is ({}) -> ({})",
        list(signature.params()),
        list(signature.results())
    )
}

/// The value of a function whose body is a single `i32.const`, as the SDK's `ito_abi_version` and
/// `ito_supported_kinds` are.
fn constant_result(body: &FunctionBody, export: &str) -> Result<u32> {
    let mut operators = body
        .get_operators_reader()
        .context("Invalid function body")?;
    let first = operators.read().context("Invalid function body")?;
    let second = operators.read().context("Invalid function body")?;
    match (first, second) {
        (Operator::I32Const { value }, Operator::End) if operators.eof() => Ok(value as u32),
        _ => bail!("{export} must return a constant"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(body: &str) -> Result<WasmInterface> {
        let bytes = wat::parse_str(format!("(module {body})"))?;
        WasmInterface::read(&bytes)
    }

    #[test]
    fn reads_the_abi_constant_and_the_imports() -> Result<()> {
        let interface = module(
            r#"
            (import "ito:core/net" "fetch_v2" (func (param i32 i32 i32 i32) (result i32)))
            (import "ito:core/html" "parse" (func (param i32 i32) (result i32)))
            (import "ito:core/net" "fetch_read" (func (param i32)))
            (func (export "ito_abi_version") (result i32) i32.const 3)
            "#,
        )?;

        assert_eq!(interface.abi(), 3);
        assert_eq!(
            interface.import_names(),
            vec![
                "ito:core/html.parse",
                "ito:core/net.fetch_read",
                "ito:core/net.fetch_v2"
            ]
        );
        assert!(interface.unknown_imports().is_empty());
        Ok(())
    }

    #[test]
    fn a_module_without_the_abi_export_is_abi_1() -> Result<()> {
        let interface = module(r#"(func (export "get_home") (result i64) i64.const 0)"#)?;
        assert_eq!(interface.abi_export, None);
        assert_eq!(interface.abi(), UNVERSIONED_ABI);
        Ok(())
    }

    #[test]
    fn imports_no_host_provides_are_named() -> Result<()> {
        let interface = module(
            r#"
            (import "ito:core/net" "fetch_v9" (func))
            (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
            (import "ito:core/std" "print" (func (param i32 i32)))
            "#,
        )?;
        assert_eq!(
            interface.unknown_imports(),
            vec!["ito:core/net.fetch_v9", "wasi_snapshot_preview1.fd_write"]
        );
        Ok(())
    }

    #[test]
    fn an_abi_export_that_is_not_a_constant_is_rejected() {
        let result = module(
            r#"
            (global $g (mut i32) (i32.const 1))
            (func (export "ito_abi_version") (result i32) global.get $g)
            "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn reads_the_kinds_a_plugin_serves() -> Result<()> {
        let interface = module(
            r#"
            (func (export "ito_abi_version") (result i32) i32.const 2)
            (func (export "ito_supported_kinds") (result i32) i32.const 5)
            "#,
        )?;
        assert_eq!(interface.kinds()?, Some(vec!["manga", "novel"]));
        let older = module(r#"(func (export "get_home") (result i64) i64.const 0)"#)?;
        assert_eq!(older.kinds()?, None);
        // Hosts read kinds from ABI 2 on, so an older module's export is not recorded.
        let early = module(r#"(func (export "ito_supported_kinds") (result i32) i32.const 4)"#)?;
        assert_eq!(early.kinds()?, None);
        Ok(())
    }

    #[test]
    fn an_abi_2_plugin_must_name_known_kinds() -> Result<()> {
        let undeclared = module(r#"(func (export "ito_abi_version") (result i32) i32.const 2)"#)?;
        assert!(undeclared.kinds().is_err());
        for mask in [0, 8, 9] {
            let interface = module(&format!(
                r#"
                (func (export "ito_abi_version") (result i32) i32.const 2)
                (func (export "ito_supported_kinds") (result i32) i32.const {mask})
                "#
            ))?;
            assert!(interface.kinds().is_err(), "mask {mask}");
        }
        let computed = module(
            r#"
            (global $g (mut i32) (i32.const 1))
            (func (export "ito_supported_kinds") (result i32) global.get $g)
            "#,
        );
        assert!(computed.is_err());
        Ok(())
    }

    #[test]
    fn a_start_section_is_detected_and_refused() -> Result<()> {
        let started = module(r#"(func $init) (start $init)"#)?;
        assert!(started.has_start_section);
        assert!(started.require_no_start_section().is_err());

        let plain = module(r#"(func $init) (func (export "get_home") (result i64) i64.const 0)"#)?;
        assert!(!plain.has_start_section);
        assert!(plain.require_no_start_section().is_ok());
        Ok(())
    }

    const ABI_3: &str = r#"
        (import "ito:core/net" "fetch_v2" (func (param i32 i32 i32 i32) (result i32)))
        (func (export "ito_abi_version") (result i32) i32.const 3)
        (func (export "ito_supported_kinds") (result i32) i32.const 1)
    "#;

    fn wasm(body: &str) -> Result<Vec<u8>> {
        Ok(wat::parse_str(format!("(module {body})"))?)
    }

    #[test]
    fn inspection_accepts_a_conforming_module_with_or_without_an_expectation() -> Result<()> {
        let bytes = wasm(ABI_3)?;
        assert_eq!(WasmInterface::inspect(&bytes, None)?.abi(), 3);
        assert_eq!(WasmInterface::inspect(&bytes, Some(3))?.abi(), 3);
        Ok(())
    }

    #[test]
    fn inspection_refuses_another_abi_than_expected() -> Result<()> {
        let error = WasmInterface::inspect(&wasm(ABI_3)?, Some(2))
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            error.as_deref(),
            Some("The plugin is built for ABI 3, but ABI 2 is expected")
        );
        Ok(())
    }

    #[test]
    fn inspection_refuses_a_start_section() -> Result<()> {
        let bytes = wasm(&format!("{ABI_3} (func $init) (start $init)"))?;
        assert!(WasmInterface::inspect(&bytes, None).is_err());
        assert!(WasmInterface::inspect(&bytes, Some(3)).is_err());
        Ok(())
    }

    #[test]
    fn inspection_refuses_an_abi_export_that_takes_a_parameter() -> Result<()> {
        let bytes = wasm(
            r#"
            (func (export "ito_abi_version") (param i32) (result i32) i32.const 3)
            (func (export "ito_supported_kinds") (result i32) i32.const 1)
            "#,
        )?;
        let error = WasmInterface::inspect(&bytes, None)
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            error.as_deref(),
            Some("ito_abi_version must be a function () -> i32, but it is (i32) -> (i32)")
        );
        Ok(())
    }

    #[test]
    fn inspection_refuses_a_kinds_export_that_returns_i64() -> Result<()> {
        let bytes = wasm(
            r#"
            (func (export "ito_abi_version") (result i32) i32.const 3)
            (func (export "ito_supported_kinds") (result i64) i64.const 1)
            "#,
        )?;
        let error = WasmInterface::inspect(&bytes, None)
            .err()
            .map(|e| e.to_string());
        assert_eq!(
            error.as_deref(),
            Some("ito_supported_kinds must be a function () -> i32, but it is () -> (i64)")
        );
        Ok(())
    }

    #[test]
    fn inspection_refuses_an_export_with_extra_results() -> Result<()> {
        let bytes = wasm(
            r#"
            (func (export "ito_abi_version") (result i32 i32) i32.const 3 i32.const 0)
            "#,
        )?;
        assert!(WasmInterface::inspect(&bytes, None).is_err());
        Ok(())
    }

    #[test]
    fn inspection_refuses_imports_no_host_provides() -> Result<()> {
        let bytes = wasm(&format!(
            r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))) {ABI_3}"#
        ))?;
        let error = WasmInterface::inspect(&bytes, None)
            .err()
            .map(|e| e.to_string());
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("wasi_snapshot_preview1.fd_write"))
        );
        Ok(())
    }

    #[test]
    fn the_expected_abi_must_match_when_given() -> Result<()> {
        let abi_3 = module(r#"(func (export "ito_abi_version") (result i32) i32.const 3)"#)?;
        assert!(abi_3.require_abi(None).is_ok());
        assert!(abi_3.require_abi(Some(3)).is_ok());
        assert!(abi_3.require_abi(Some(2)).is_err());

        // A module without the export is ABI 1 and is held to the expectation like any other.
        let unversioned = module(r#"(func (export "get_home") (result i64) i64.const 0)"#)?;
        assert!(unversioned.require_abi(Some(1)).is_ok());
        assert!(unversioned.require_abi(Some(3)).is_err());
        Ok(())
    }
}
