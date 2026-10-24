//! What a plugin's WebAssembly module needs from the host, and the ABI it was built for.

use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use wasmparser::{ExternalKind, FunctionBody, Operator, Parser, Payload, TypeRef};

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
}

impl WasmInterface {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut imports = Vec::new();
        let mut imported_functions = 0u32;
        let mut defined_functions = 0u32;
        let mut abi_function = None;
        let mut abi_export = None;
        let mut kinds_function = None;
        let mut kinds_export = None;

        for payload in Parser::new(0).parse_all(bytes) {
            match payload.context("Invalid WebAssembly module")? {
                Payload::ImportSection(reader) => {
                    for import in reader.into_imports() {
                        let import = import.context("Invalid import")?;
                        if matches!(import.ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) {
                            imported_functions += 1;
                        }
                        imports.push((import.module.to_string(), import.name.to_string()));
                    }
                }
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.context("Invalid export")?;
                        if export.kind == ExternalKind::Func {
                            match export.name {
                                ABI_EXPORT => abi_function = Some(export.index),
                                KINDS_EXPORT => kinds_function = Some(export.index),
                                _ => {}
                            }
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
        })
    }

    /// The ABI the plugin was built for: its `ito_abi_version`, or 1 without one.
    pub fn abi(&self) -> u32 {
        self.abi_export.unwrap_or(UNVERSIONED_ABI)
    }

    /// The kinds the plugin serves, in `KINDS` order: from `ito_supported_kinds`, which ABI 2
    /// plugins must export. None for older plugins, which serve their manifest `type`.
    pub fn kinds(&self) -> Result<Option<Vec<&'static str>>> {
        let Some(mask) = self.kinds_export else {
            if self.abi() >= 2 {
                bail!("An ABI {} plugin must export {KINDS_EXPORT}", self.abi());
            }
            return Ok(None);
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
}
