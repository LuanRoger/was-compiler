use js_sys::{Array, Error};
use wasm_bindgen::prelude::*;

use was_compiler::{
    CodegenError, CompileError, Compiler, ExportOptions, HostRegistrationError, OptimizationLevel,
    Type, diagnostic,
};

#[wasm_bindgen(js_name = Compiler)]
pub struct BrowserCompiler {
    compiler: Compiler,
}

#[wasm_bindgen(js_class = Compiler)]
impl BrowserCompiler {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            compiler: Compiler::new(),
        }
    }

    pub fn compile(&self, source: &str) -> Result<Vec<u8>, JsValue> {
        self.compiler.compile_source(source).map_err(|error| {
            let range_export = matches!(
                error,
                CompileError::Codegen(CodegenError::RangeExport { .. })
            );
            let mut message = diagnostic::render("<browser>", source, &error);

            if range_export {
                message.push_str(
                    "\nSelect only compatible functions with setExports([\"main\"]), \
                     or use exportNone() to keep all user functions internal.",
                );
            }

            js_error(&message)
        })
    }

    #[wasm_bindgen(js_name = compileWat)]
    pub fn compile_wat(&self, source: &str) -> Result<String, JsValue> {
        let bytes = self.compile(source)?;

        wasmprinter::print_bytes(bytes)
            .map_err(|error| js_error(&format!("WAT serialization error: {error}")))
    }

    #[wasm_bindgen(js_name = registerHostFunction)]
    pub fn register_host_function(
        &mut self,
        name: &str,
        module: &str,
        field: &str,
        #[wasm_bindgen(unchecked_param_type = "string[]")] params: Array,
        result: &str,
    ) -> Result<(), JsValue> {
        let params = strings(&params, "host parameter types")?
            .iter()
            .map(|name| parse_type(name))
            .collect::<Result<Vec<_>, _>>()?;
        let result = parse_type(result)?;

        self.compiler
            .register_host_function(name, module, field, params, result)
            .map(|_| ())
            .map_err(|error| {
                let mut message = format!("host registration error: {error}");

                if matches!(
                    error,
                    HostRegistrationError::UnsupportedType {
                        ty: Type::Range,
                        ..
                    }
                ) {
                    message.push_str(
                        "; range values are private and cannot cross host imports; \
                         use number, bool, string, number[], bool[], string[] parameters/results or void results",
                    );
                }

                js_error(&message)
            })
    }

    #[wasm_bindgen(js_name = setExports)]
    pub fn set_exports(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "string[]")] names: Array,
    ) -> Result<(), JsValue> {
        let names = strings(&names, "export names")?;

        self.compiler.set_exports(ExportOptions::Only(names));

        Ok(())
    }

    #[wasm_bindgen(js_name = exportAll)]
    pub fn export_all(&mut self) {
        self.compiler.set_exports(ExportOptions::All);
    }

    #[wasm_bindgen(js_name = exportNone)]
    pub fn export_none(&mut self) {
        self.compiler.set_exports(ExportOptions::None);
    }

    #[wasm_bindgen(js_name = setOptimizationLevel)]
    pub fn set_optimization_level(&mut self, level: f64) -> Result<(), JsValue> {
        let level = match level {
            0.0 => OptimizationLevel::None,
            1.0 => OptimizationLevel::Basic,
            2.0 => OptimizationLevel::Standard,
            3.0 => OptimizationLevel::Aggressive,
            _ => {
                return Err(js_error(
                    "optimization level must be an integer from 0 to 3",
                ));
            }
        };

        self.compiler.set_optimization_level(level);

        Ok(())
    }
}

impl Default for BrowserCompiler {
    fn default() -> Self {
        Self::new()
    }
}

fn strings(values: &Array, label: &str) -> Result<Vec<String>, JsValue> {
    if !Array::is_array(values.as_ref()) {
        return Err(js_error(&format!("{label} must be an array of strings")));
    }

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_string()
                .ok_or_else(|| js_error(&format!("{label}[{index}] must be a string")))
        })
        .collect()
}

fn parse_type(name: &str) -> Result<Type, JsValue> {
    match name {
        "number" => Ok(Type::Number),
        "bool" => Ok(Type::Bool),
        "string" => Ok(Type::String),
        "number[]" => Ok(Type::Array(was_compiler::ArrayElementType::Number)),
        "bool[]" => Ok(Type::Array(was_compiler::ArrayElementType::Bool)),
        "string[]" => Ok(Type::Array(was_compiler::ArrayElementType::String)),
        "void" => Ok(Type::Void),
        "range" => Ok(Type::Range),
        _ => Err(js_error(&format!(
            "unknown type {name:?}; expected number, bool, string, number[], bool[], string[], or void"
        ))),
    }
}

fn js_error(message: &str) -> JsValue {
    Error::new(message).into()
}
