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
    ("ito:core/defaults", &["set", "get", "remove"]),
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

/// The ABI a plugin built before `ito_abi_version` existed was built for.
pub const UNVERSIONED_ABI: u32 = 1;

pub struct WasmInterface {
    /// Every import, as (module, name).
    pub imports: Vec<(String, String)>,
    /// The constant `ito_abi_version` returns, if the module exports it.
    pub abi_export: Option<u32>,
}

impl WasmInterface {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut imports = Vec::new();
        let mut imported_functions = 0u32;
        let mut defined_functions = 0u32;
        let mut abi_function = None;
        let mut abi_export = None;

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
                        if export.name == ABI_EXPORT && export.kind == ExternalKind::Func {
                            abi_function = Some(export.index);
                        }
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    if abi_function == Some(imported_functions + defined_functions) {
                        abi_export = Some(constant_result(&body)?);
                    }
                    defined_functions += 1;
                }
                _ => {}
            }
        }

        if abi_function.is_some() && abi_export.is_none() {
            bail!("{ABI_EXPORT} must be defined in the module, not imported");
        }
        Ok(Self {
            imports,
            abi_export,
        })
    }

    /// The ABI the plugin was built for: its `ito_abi_version`, or 1 without one.
    pub fn abi(&self) -> u32 {
        self.abi_export.unwrap_or(UNVERSIONED_ABI)
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

/// The value of a function whose body is a single `i32.const`, as the SDK's `ito_abi_version` is.
fn constant_result(body: &FunctionBody) -> Result<u32> {
    let mut operators = body
        .get_operators_reader()
        .context("Invalid function body")?;
    let first = operators.read().context("Invalid function body")?;
    let second = operators.read().context("Invalid function body")?;
    match (first, second) {
        (Operator::I32Const { value }, Operator::End) if operators.eof() => Ok(value as u32),
        _ => bail!("{ABI_EXPORT} must return a constant"),
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
}
