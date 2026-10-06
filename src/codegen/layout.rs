use std::collections::{HashMap, HashSet};

use crate::typed_ast::TypedProgram;

use super::{
    CodegenError,
    abi::{self, Signature},
    checked_u32,
};

pub(super) struct UserFunctionPlan {
    pub type_index: u32,
    pub function_index: u32,
}

pub(super) struct Layout {
    pub signatures: Vec<Signature>,
    pub host_type_indices: Vec<u32>,
    pub users: Vec<UserFunctionPlan>,
}

impl Layout {
    pub fn new(program: &TypedProgram) -> Result<Self, CodegenError> {
        let host_count = checked_u32(program.host_functions.len(), "host function count")?;
        let user_count = checked_u32(program.functions.len(), "user function count")?;

        host_count
            .checked_add(user_count)
            .ok_or(CodegenError::LimitExceeded {
                what: "total function count",
            })?;

        let mut layout = Self {
            signatures: Vec::new(),
            host_type_indices: Vec::new(),
            users: Vec::new(),
        };

        let mut type_indices = HashMap::new();
        let mut export_names = HashSet::new();

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
        }

        for (expected, user) in program.functions.iter().enumerate() {
            if user.id.0 != expected {
                return Err(CodegenError::InvalidFunctionId {
                    function: user.name.clone(),
                    expected,
                    actual: user.id.0,
                });
            }

            if !export_names.insert(user.name.as_str()) {
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
            });
        }

        Ok(layout)
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
