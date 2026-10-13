use thiserror::Error;

use crate::ast::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostFunctionId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub struct HostFunction {
    pub id: HostFunctionId,
    pub name: String,
    pub module: String,
    pub field: String,
    pub params: Vec<Type>,
    pub return_type: Type,
}

#[derive(Debug, Error, PartialEq)]
pub enum HostRegistrationError {
    #[error("Invalid host function name '{name}'")]
    InvalidName { name: String },

    #[error("Host function name '{name}' is reserved")]
    ReservedName { name: String },

    #[error("Host function '{name}' is already registered")]
    DuplicateName { name: String },

    #[error("Host import module cannot be empty")]
    EmptyModule,

    #[error("Host import field cannot be empty")]
    EmptyField,

    #[error("Host function '{name}' uses unsupported ABI type {ty:?}")]
    UnsupportedType { name: String, ty: Type },
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct HostFunctions {
    functions: Vec<HostFunction>,
}

// External eligibility is independent of the backend's internal representation.
pub(crate) fn supports_external_value(ty: Type) -> bool {
    match ty {
        Type::Number | Type::Bool | Type::String | Type::NumberArray => true,
        Type::Void | Type::Range => false,
    }
}

pub(crate) fn supports_external_result(ty: Type) -> bool {
    ty == Type::Void || supports_external_value(ty)
}

impl HostFunctions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        module: impl Into<String>,
        field: impl Into<String>,
        params: Vec<Type>,
        return_type: Type,
    ) -> Result<HostFunctionId, HostRegistrationError> {
        let name = name.into();
        let module = module.into();
        let field = field.into();
        let bytes = name.as_bytes();

        if !bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return Err(HostRegistrationError::InvalidName { name });
        }

        if matches!(
            name.as_str(),
            "fn" | "let"
                | "if"
                | "else"
                | "while"
                | "for"
                | "loop"
                | "break"
                | "continue"
                | "return"
                | "requires"
                | "true"
                | "false"
                | "number"
                | "bool"
                | "string"
                | "void"
                | "range"
                | "in"
                | "len"
        ) {
            return Err(HostRegistrationError::ReservedName { name });
        }

        if self.functions.iter().any(|function| function.name == name) {
            return Err(HostRegistrationError::DuplicateName { name });
        }

        if module.is_empty() {
            return Err(HostRegistrationError::EmptyModule);
        }

        if field.is_empty() {
            return Err(HostRegistrationError::EmptyField);
        }

        for ty in &params {
            if !supports_external_value(*ty) {
                return Err(HostRegistrationError::UnsupportedType { name, ty: *ty });
            }
        }

        if !supports_external_result(return_type) {
            return Err(HostRegistrationError::UnsupportedType {
                name,
                ty: return_type,
            });
        }

        let id = HostFunctionId(self.functions.len());

        self.functions.push(HostFunction {
            id,
            name,
            module,
            field,
            params,
            return_type,
        });

        Ok(id)
    }

    pub fn functions(&self) -> &[HostFunction] {
        &self.functions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_preserves_names_signatures_and_order() {
        let mut functions = HostFunctions::new();

        assert_eq!(
            functions.register("print", "env", "log", vec![Type::Number], Type::Void),
            Ok(HostFunctionId(0))
        );
        assert_eq!(
            functions.register("ready", "app", "is_ready", vec![], Type::Bool),
            Ok(HostFunctionId(1))
        );

        assert_eq!(
            functions.functions()[0],
            HostFunction {
                id: HostFunctionId(0),
                name: "print".into(),
                module: "env".into(),
                field: "log".into(),
                params: vec![Type::Number],
                return_type: Type::Void,
            }
        );
    }

    #[test]
    fn invalid_registration_does_not_consume_an_id() {
        let mut functions = HostFunctions::new();

        for name in ["", "1name", "has space", "é", "a-b"] {
            assert_eq!(
                functions.register(name, "env", "run", vec![], Type::Void),
                Err(HostRegistrationError::InvalidName { name: name.into() })
            );
        }

        for name in [
            "fn", "let", "if", "else", "while", "return", "true", "false", "number", "bool",
            "string", "void", "range", "len", "break", "continue", "loop",
        ] {
            assert_eq!(
                functions.register(name, "env", "run", vec![], Type::Void),
                Err(HostRegistrationError::ReservedName { name: name.into() })
            );
        }

        assert_eq!(
            functions.register("run", "", "run", vec![], Type::Void),
            Err(HostRegistrationError::EmptyModule)
        );
        assert_eq!(
            functions.register("run", "env", "", vec![], Type::Void),
            Err(HostRegistrationError::EmptyField)
        );

        assert!(functions.functions().is_empty());
        assert_eq!(
            functions.register("run", "env", "run", vec![], Type::Void),
            Ok(HostFunctionId(0))
        );
        assert_eq!(
            functions.register("run", "other", "run", vec![], Type::Void),
            Err(HostRegistrationError::DuplicateName { name: "run".into() })
        );
        assert_eq!(
            functions.register("_next2", "env", "run", vec![], Type::Void),
            Ok(HostFunctionId(1))
        );
    }

    #[test]
    fn loop_keywords_reserve_only_language_names_not_import_names() {
        let mut functions = HostFunctions::new();

        for name in ["break", "continue", "loop"] {
            assert_eq!(
                functions.register(name, "env", "run", vec![], Type::Void),
                Err(HostRegistrationError::ReservedName { name: name.into() })
            );
        }

        for (index, (name, module, field)) in [
            ("break1", "break", "continue"),
            ("continued", "continue", "break"),
            ("loop1", "loop", "loop"),
            ("Loop", "loop", "continue"),
            ("Break", "module with spaces/λ", "field:with,delimiters"),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                functions.register(name, module, field, vec![], Type::Void),
                Ok(HostFunctionId(index))
            );
            assert_eq!(functions.functions()[index].module, module);
            assert_eq!(functions.functions()[index].field, field);
        }
    }

    #[test]
    fn void_parameters_are_rejected_at_registration() {
        let mut functions = HostFunctions::new();

        assert_eq!(
            functions.register("run", "env", "run", vec![Type::Void], Type::Void),
            Err(HostRegistrationError::UnsupportedType {
                name: "run".into(),
                ty: Type::Void,
            })
        );

        assert!(functions.functions().is_empty());
    }

    #[test]
    fn external_signatures_accept_all_current_values_and_void_results() {
        let mut functions = HostFunctions::new();
        let params = vec![Type::Number, Type::Bool, Type::String, Type::NumberArray];

        for (index, return_type) in [
            Type::Number,
            Type::Bool,
            Type::String,
            Type::NumberArray,
            Type::Void,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                functions.register(
                    format!("host{index}"),
                    "env",
                    "call",
                    params.clone(),
                    return_type,
                ),
                Ok(HostFunctionId(index))
            );
            assert_eq!(functions.functions()[index].params, params);
            assert_eq!(functions.functions()[index].return_type, return_type);
        }

        for index in 0..=params.len() {
            let mut invalid = params.clone();
            invalid.insert(index, Type::Void);

            let error = functions
                .register("bad", "env", "call", invalid, Type::Void)
                .unwrap_err();

            assert_eq!(
                error,
                HostRegistrationError::UnsupportedType {
                    name: "bad".into(),
                    ty: Type::Void,
                }
            );
            assert_eq!(
                error.to_string(),
                "Host function 'bad' uses unsupported ABI type Void"
            );
            assert_eq!(functions.functions().len(), 5);
        }
    }

    #[test]
    fn memory_backed_host_signatures_preserve_language_types() {
        let mut functions = HostFunctions::new();

        for (index, ty) in [Type::String, Type::NumberArray].into_iter().enumerate() {
            let name = format!("echo{index}");

            assert_eq!(
                functions.register(&name, "env", &name, vec![ty], ty),
                Ok(HostFunctionId(index))
            );
            assert_eq!(functions.functions()[index].params, vec![ty]);
            assert_eq!(functions.functions()[index].return_type, ty);
        }
    }
}
