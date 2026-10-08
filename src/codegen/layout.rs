use std::collections::{HashMap, HashSet};

use crate::{
    ast::Type,
    exports::ExportOptions,
    typed_ast::{FunctionRef, TypedProgram},
};

use super::{
    CodegenError,
    abi::{self, Signature},
    checked_u32,
    memory::{self, MemoryPlan},
    numeric,
};

pub(super) struct UserFunctionPlan {
    pub type_index: u32,
    pub function_index: u32,
    pub exported: bool,
}

pub(super) struct CallTarget {
    pub function_index: u32,
    pub params: Vec<Type>,
    pub return_type: Type,
}

pub(super) struct MemoryLayout {
    pub plan: MemoryPlan,
    pub helpers: Vec<UserFunctionPlan>,
    pub first_function: u32,
}

pub(super) struct Layout {
    pub signatures: Vec<Signature>,
    pub host_type_indices: Vec<u32>,
    pub users: Vec<UserFunctionPlan>,
    pub memory: Option<MemoryLayout>,
    pub remainder: Option<UserFunctionPlan>,
    callees: Vec<CallTarget>,
    host_count: usize,
}

impl Layout {
    pub fn new(program: &TypedProgram) -> Result<Self, CodegenError> {
        Self::with_exports(program, &ExportOptions::All)
    }

    pub fn with_exports(
        program: &TypedProgram,
        exports: &ExportOptions,
    ) -> Result<Self, CodegenError> {
        let host_count = checked_u32(program.host_functions.len(), "host function count")?;
        let user_count = checked_u32(program.functions.len(), "user function count")?;

        let first_runtime =
            host_count
                .checked_add(user_count)
                .ok_or(CodegenError::LimitExceeded {
                    what: "total function count",
                })?;

        let mut layout = Self {
            signatures: Vec::new(),
            host_type_indices: Vec::new(),
            users: Vec::new(),
            memory: None,
            remainder: None,
            callees: Vec::new(),
            host_count: program.host_functions.len(),
        };

        let mut type_indices = HashMap::new();
        let mut user_names = HashSet::new();

        for (expected, host) in program.host_functions.iter().enumerate() {
            if host.id.0 != expected {
                return Err(CodegenError::InvalidHostFunctionId {
                    function: host.name.clone(),
                    expected,
                    actual: host.id.0,
                });
            }

            checked_u32(host.module.len(), "import module name length")?;
            checked_u32(host.field.len(), "import field name length")?;

            let signature =
                abi::signature(&host.name, host.params.iter().copied(), host.return_type)?;
            let type_index = layout.intern(signature, &mut type_indices)?;

            layout.host_type_indices.push(type_index);
            layout.callees.push(CallTarget {
                function_index: checked_u32(expected, "host function index")?,
                params: host.params.clone(),
                return_type: host.return_type,
            });
        }

        for (expected, user) in program.functions.iter().enumerate() {
            if user.id.0 != expected {
                return Err(CodegenError::InvalidFunctionId {
                    function: user.name.clone(),
                    expected,
                    actual: user.id.0,
                });
            }

            if !user_names.insert(user.name.as_str()) {
                return Err(CodegenError::DuplicateExportName {
                    name: user.name.clone(),
                });
            }

            checked_u32(user.name.len(), "export name length")?;

            let signature = abi::signature(
                &user.name,
                user.params.iter().map(|param| param.ty),
                user.return_type,
            )?;

            let type_index = layout.intern(signature, &mut type_indices)?;
            let function_index = host_count
                .checked_add(checked_u32(expected, "user function index")?)
                .ok_or(CodegenError::LimitExceeded {
                    what: "function index",
                })?;

            layout.users.push(UserFunctionPlan {
                type_index,
                function_index,
                exported: false,
            });

            layout.callees.push(CallTarget {
                function_index,
                params: user.params.iter().map(|param| param.ty).collect(),
                return_type: user.return_type,
            });
        }

        let export_names = match exports {
            ExportOptions::All => user_names,
            ExportOptions::None => HashSet::new(),
            ExportOptions::Only(names) => {
                let mut selected = HashSet::new();

                for name in names {
                    if !selected.insert(name.as_str()) {
                        return Err(CodegenError::DuplicateExportSelection { name: name.clone() });
                    }

                    if !user_names.contains(name.as_str()) {
                        return Err(CodegenError::UnknownExport { name: name.clone() });
                    }
                }

                selected
            }
        };

        for (user, plan) in program.functions.iter().zip(&mut layout.users) {
            plan.exported = export_names.contains(user.name.as_str());
        }

        if let Some(plan) = MemoryPlan::new(program)? {
            for name in ["memory", "__alloc"] {
                if export_names.contains(name) {
                    return Err(CodegenError::DuplicateExportName { name: name.into() });
                }
            }

            let helpers = memory::helpers();

            first_runtime
                .checked_add(checked_u32(helpers.len(), "runtime function count")?)
                .ok_or(CodegenError::LimitExceeded {
                    what: "total function count",
                })?;

            let mut plans = Vec::with_capacity(helpers.len());

            for (helper, signature) in helpers {
                plans.push(UserFunctionPlan {
                    type_index: layout.intern(signature, &mut type_indices)?,
                    function_index: first_runtime + helper.index(),
                    exported: false,
                });
            }

            layout.memory = Some(MemoryLayout {
                plan,
                helpers: plans,
                first_function: first_runtime,
            });
        }

        if numeric::needs_remainder(program) {
            let memory_helpers = layout
                .memory
                .as_ref()
                .map_or(0, |memory| memory.helpers.len());
            let function_index = first_runtime
                .checked_add(checked_u32(memory_helpers, "runtime function count")?)
                .ok_or(CodegenError::LimitExceeded {
                    what: "total function count",
                })?;

            function_index
                .checked_add(1)
                .ok_or(CodegenError::LimitExceeded {
                    what: "total function count",
                })?;

            let signature = Signature {
                params: vec![wasm_encoder::ValType::F64, wasm_encoder::ValType::F64],
                result: Some(wasm_encoder::ValType::F64),
            };

            layout.remainder = Some(UserFunctionPlan {
                type_index: layout.intern(signature, &mut type_indices)?,
                function_index,
                exported: false,
            });
        }

        Ok(layout)
    }

    pub fn resolve(&self, reference: FunctionRef) -> Option<&CallTarget> {
        match reference {
            FunctionRef::Host(id) if id.0 < self.host_count => self.callees.get(id.0),

            FunctionRef::User(id) => {
                self.users.get(id.0)?;

                self.callees.get(self.host_count.checked_add(id.0)?)
            }

            FunctionRef::Host(_) | FunctionRef::Builtin(_) => None,
        }
    }

    fn intern(
        &mut self,
        signature: Signature,
        type_indices: &mut HashMap<Signature, u32>,
    ) -> Result<u32, CodegenError> {
        if let Some(index) = type_indices.get(&signature) {
            return Ok(*index);
        }

        let index = checked_u32(self.signatures.len(), "function type index")?;

        type_indices.insert(signature.clone(), index);
        self.signatures.push(signature);

        Ok(index)
    }
}
