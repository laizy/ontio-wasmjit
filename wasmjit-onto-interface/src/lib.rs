#![cfg_attr(
    feature = "cargo-clippy",
    allow(clippy::missing_safety_doc, clippy::new_without_default)
)]
#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use ontio_wasmjit::chain_api::{
    check_gas_and_host_panic, convert_chainctx, ChainCtx, ChainResolver, CALL_CONTRACT_GAS,
    CONTRACT_CREATE_GAS, PER_UNIT_CODE_LEN, STORAGE_DELETE_GAS, STORAGE_GET_GAS, STORAGE_PUT_GAS,
    UINT_DEPLOY_CODE_LEN_GAS,
};
use ontio_wasmjit::executor::Instance;
pub use ontio_wasmjit::resolver::Resolver;
use ontio_wasmjit_runtime::builtins::{check_host_panic, check_internel_panic};
use ontio_wasmjit_runtime::{wasmjit_unwind, VMContext, VMFunctionBody, VMFunctionImport};
use std::ptr;
pub use wasmjit_capi::{
    address_t, bytes_from_vec, bytes_null, bytes_to_boxed_slice, convert_chain_ctx, convert_vmctx,
    wasmjit_bytes_new, wasmjit_bytes_t, wasmjit_chain_context_get_exec_step,
    wasmjit_chain_context_get_gas, wasmjit_chain_context_set_calloutput,
    wasmjit_chain_context_set_exec_step, wasmjit_chain_context_set_gas, wasmjit_chain_context_t,
    wasmjit_chain_context_take_output, wasmjit_instance_destroy, wasmjit_instance_invoke,
    wasmjit_instance_t, wasmjit_instantiate, wasmjit_resolver_t, wasmjit_result_err_internal,
    wasmjit_result_err_trap, wasmjit_result_success, wasmjit_result_t, wasmjit_slice_t,
    wasmjit_vmctx_chainctx, wasmjit_vmctx_memory, wasmjit_vmctx_t,
};

#[repr(C)]
pub struct wasmjit_u32 {
    v: u32,
    res: wasmjit_result_t,
}

#[repr(C)]
pub struct wasmjit_u64 {
    v: u64,
    res: wasmjit_result_t,
}

#[repr(C)]
pub struct wasmjit_ret {
    exec_step: u64,
    gas_left: u64,
    buffer: wasmjit_bytes_t,
    res: wasmjit_result_t,
}

extern "C" {
    fn ontio_debug_cgo(data: wasmjit_slice_t);
    fn ontio_notify_cgo(service_index: u64, data: wasmjit_slice_t) -> wasmjit_result_t;
    fn ontio_storage_read_cgo(
        service_index: u64,
        key: wasmjit_slice_t,
        val: wasmjit_slice_t,
        offset: u32,
    ) -> wasmjit_u32;
    fn ontio_storage_write_cgo(service_index: u64, key: wasmjit_slice_t, val: wasmjit_slice_t);
    fn ontio_storage_delete_cgo(service_index: u64, key: wasmjit_slice_t);
    fn ontio_contract_create_cgo(
        service_index: u64,
        code: wasmjit_slice_t,
        vm_type: u32,
        name: wasmjit_slice_t,
        ver: wasmjit_slice_t,
        author: wasmjit_slice_t,
        email: wasmjit_slice_t,
        desc: wasmjit_slice_t,
        newaddress_ptr: &mut address_t,
    ) -> wasmjit_result_t;
    fn ontio_contract_migrate_cgo(
        service_index: u64,
        code: wasmjit_slice_t,
        vm_type: u32,
        name: wasmjit_slice_t,
        ver: wasmjit_slice_t,
        author: wasmjit_slice_t,
        email: wasmjit_slice_t,
        desc: wasmjit_slice_t,
        newaddress_ptr: &mut address_t,
    ) -> wasmjit_result_t;
    fn ontio_contract_destroy_cgo(service_index: u64) -> wasmjit_result_t;
    fn ontio_call_contract_cgo(
        vmctx: *mut wasmjit_vmctx_t,
        contract_addr: &mut address_t,
        input: wasmjit_slice_t,
    ) -> wasmjit_result_t;
}

unsafe fn check_wasmjit_result(res: wasmjit_result_t) -> Result<(), String> {
    match res.kind {
        wasmjit_result_success => Ok(()),
        wasmjit_result_err_trap => {
            Err((std::string::String::from_utf8_lossy(&bytes_to_boxed_slice(res.msg))).to_string())
        }
        _ => panic!( // must be internel err
                (std::string::String::from_utf8_lossy(&bytes_to_boxed_slice(res.msg))).to_string()
            ),
    }
}

/// Implementation of ontio_debug api
#[no_mangle]
pub unsafe extern "C" fn ontio_debug(vmctx: *mut VMContext, data_ptr: u32, l: u32) {
    check_host_panic((&mut *vmctx).instance(), |_instance| {
        let data = wasm_pointer_to_jit_slice(vmctx, data_ptr, l)?;
        ontio_debug_cgo(data);
        Ok(())
    });
}

/// Implementation of ontio_call_contract
#[no_mangle]
pub unsafe extern "C" fn ontio_call_contract(
    vmctx: *mut VMContext,
    contract_addr: u32,
    input_ptr: u32,
    inputlen: u32,
) -> u32 {
    check_gas_and_host_panic((&mut *vmctx).instance(), CALL_CONTRACT_GAS, |_instance| {
        let input = wasm_pointer_to_jit_slice(vmctx, input_ptr, inputlen)?;
        let addr = wasm_pointer_to_jit_slice(vmctx, contract_addr, 20)?;

        let res = ontio_call_contract_cgo(
            vmctx as *mut wasmjit_vmctx_t,
            &mut *(addr.data as *mut address_t),
            input,
        );

        check_wasmjit_result(res)?;

        let host = (&mut *vmctx).host_state();
        Ok(convert_chainctx(host).call_output_len())
    })
}

/// Implementation of ontio_notify api
#[no_mangle]
pub unsafe extern "C" fn ontio_notify(vmctx: *mut VMContext, ptr: u32, l: u32) {
    check_host_panic((&mut *vmctx).instance(), |_| {
        let data = wasm_pointer_to_jit_slice(vmctx, ptr, l)?;

        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        let res = ontio_notify_cgo(service_index, data);
        check_wasmjit_result(res)?;
        Ok(())
    })
}

/// Implementation of ontio_storage_read
#[no_mangle]
pub unsafe extern "C" fn ontio_storage_read(
    vmctx: *mut VMContext,
    key_ptr: u32,
    klen: u32,
    val: u32,
    vlen: u32,
    offset: u32,
) -> u32 {
    check_gas_and_host_panic((&mut *vmctx).instance(), STORAGE_GET_GAS, |_| {
        let key = wasm_pointer_to_jit_slice(vmctx, key_ptr, klen)?;
        let value = wasm_pointer_to_jit_slice(vmctx, val, vlen)?;

        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        let jit_u32 = ontio_storage_read_cgo(service_index, key, value, offset);

        check_wasmjit_result(jit_u32.res)?;
        Ok(jit_u32.v)
    })
}

/// Implementation of ontio_storage_write
#[no_mangle]
pub unsafe extern "C" fn ontio_storage_write(
    vmctx: *mut VMContext,
    key_ptr: u32,
    klen: u32,
    val: u32,
    vlen: u32,
) {
    let costs = if klen + vlen != 0 {
        (((klen + vlen) + 1023) / 1024) as u64 * STORAGE_PUT_GAS
    } else {
        STORAGE_PUT_GAS
    };
    check_gas_and_host_panic((&mut *vmctx).instance(), costs, |_| {
        let key = wasm_pointer_to_jit_slice(vmctx, key_ptr, klen)?;
        let value = wasm_pointer_to_jit_slice(vmctx, val, vlen)?;
        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        ontio_storage_write_cgo(service_index, key, value);
        Ok(())
    });
}

/// Implementation of ontio_storage_delete
#[no_mangle]
pub unsafe extern "C" fn ontio_storage_delete(vmctx: *mut VMContext, key_ptr: u32, klen: u32) {
    check_gas_and_host_panic((&mut *vmctx).instance(), STORAGE_DELETE_GAS, |_| {
        let key = wasm_pointer_to_jit_slice(vmctx, key_ptr, klen)?;
        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        ontio_storage_delete_cgo(service_index, key);
        Ok(())
    });
}

/// Implementation of ontio_contract_create
#[no_mangle]
pub unsafe extern "C" fn ontio_contract_create(
    vmctx: *mut VMContext,
    code_ptr: u32,
    code_len: u32,
    vm_type: u32,
    name_ptr: u32,
    name_len: u32,
    ver_ptr: u32,
    ver_len: u32,
    author_ptr: u32,
    author_len: u32,
    email_ptr: u32,
    email_len: u32,
    desc_ptr: u32,
    desc_len: u32,
    newaddress_ptr: u32,
) -> u32 {
    let costs =
        CONTRACT_CREATE_GAS + ((code_len as u64) / PER_UNIT_CODE_LEN) * UINT_DEPLOY_CODE_LEN_GAS;

    check_gas_and_host_panic((&mut *vmctx).instance(), costs, |_| {
        let code = wasm_pointer_to_jit_slice(vmctx, code_ptr, code_len)?;
        let name = wasm_pointer_to_jit_slice(vmctx, name_ptr, name_len)?;
        let ver = wasm_pointer_to_jit_slice(vmctx, ver_ptr, ver_len)?;
        let author = wasm_pointer_to_jit_slice(vmctx, author_ptr, author_len)?;
        let email = wasm_pointer_to_jit_slice(vmctx, email_ptr, email_len)?;
        let desc = wasm_pointer_to_jit_slice(vmctx, desc_ptr, desc_len)?;
        let addr = wasm_pointer_to_jit_slice(vmctx, newaddress_ptr, 20)?;

        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        let res = ontio_contract_create_cgo(
            service_index,
            code,
            vm_type,
            name,
            ver,
            author,
            email,
            desc,
            &mut *(addr.data as *mut address_t),
        );

        check_wasmjit_result(res)?;

        Ok(20)
    })
}

/// Implementation of ontio_contract_migrate
#[no_mangle]
pub unsafe extern "C" fn ontio_contract_migrate(
    vmctx: *mut VMContext,
    code_ptr: u32,
    code_len: u32,
    vm_type: u32,
    name_ptr: u32,
    name_len: u32,
    ver_ptr: u32,
    ver_len: u32,
    author_ptr: u32,
    author_len: u32,
    email_ptr: u32,
    email_len: u32,
    desc_ptr: u32,
    desc_len: u32,
    newaddress_ptr: u32,
) -> u32 {
    let costs =
        CONTRACT_CREATE_GAS + ((code_len as u64) / PER_UNIT_CODE_LEN) * UINT_DEPLOY_CODE_LEN_GAS;

    check_gas_and_host_panic((&mut *vmctx).instance(), costs, |_| {
        let code = wasm_pointer_to_jit_slice(vmctx, code_ptr, code_len)?;
        let name = wasm_pointer_to_jit_slice(vmctx, name_ptr, name_len)?;
        let ver = wasm_pointer_to_jit_slice(vmctx, ver_ptr, ver_len)?;
        let author = wasm_pointer_to_jit_slice(vmctx, author_ptr, author_len)?;
        let email = wasm_pointer_to_jit_slice(vmctx, email_ptr, email_len)?;
        let desc = wasm_pointer_to_jit_slice(vmctx, desc_ptr, desc_len)?;
        let addr = wasm_pointer_to_jit_slice(vmctx, newaddress_ptr, 20)?;

        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        let res = ontio_contract_migrate_cgo(
            service_index,
            code,
            vm_type,
            name,
            ver,
            author,
            email,
            desc,
            &mut *(addr.data as *mut address_t),
        );

        check_wasmjit_result(res)?;
        Ok(20)
    })
}

/// Implementation of ontio_contract_destroy
#[no_mangle]
pub unsafe extern "C" fn ontio_contract_destroy(vmctx: *mut VMContext) {
    check_host_panic((&mut *vmctx).instance(), |_| {
        let service_index = wasmjit_service_index(vmctx as *mut wasmjit_vmctx_t);
        let res = ontio_contract_destroy_cgo(service_index);
        check_wasmjit_result(res)?;

        let ctx = wasmjit_vmctx_chainctx(vmctx as *mut wasmjit_vmctx_t);
        let ctx_r = convert_chain_ctx(ctx);
        ctx_r.set_from_return();
        Ok(())
    });

    wasmjit_unwind(String::new());
}

pub struct OntoChainResolver {
    inner: ChainResolver,
}

impl OntoChainResolver {
    pub fn new() -> Self {
        let cr = ChainResolver;
        Self { inner: cr }
    }
}

impl Resolver for OntoChainResolver {
    fn resolve(&mut self, module: &str, field: &str) -> Option<VMFunctionImport> {
        match field {
            "ontio_debug" => Some(VMFunctionImport {
                body: ontio_debug as *const VMFunctionBody,
            }),
            "ontio_call_contract" => Some(VMFunctionImport {
                body: ontio_call_contract as *const VMFunctionBody,
            }),
            "ontio_notify" => Some(VMFunctionImport {
                body: ontio_notify as *const VMFunctionBody,
            }),
            "ontio_storage_read" => Some(VMFunctionImport {
                body: ontio_storage_read as *const VMFunctionBody,
            }),
            "ontio_storage_write" => Some(VMFunctionImport {
                body: ontio_storage_write as *const VMFunctionBody,
            }),
            "ontio_storage_delete" => Some(VMFunctionImport {
                body: ontio_storage_delete as *const VMFunctionBody,
            }),
            "ontio_contract_create" => Some(VMFunctionImport {
                body: ontio_contract_create as *const VMFunctionBody,
            }),
            "ontio_contract_migrate" => Some(VMFunctionImport {
                body: ontio_contract_migrate as *const VMFunctionBody,
            }),
            "ontio_contract_destroy" => Some(VMFunctionImport {
                body: ontio_contract_destroy as *const VMFunctionBody,
            }),
            _ => self.inner.resolve(module, field),
        }
    }
}

/// Implementation of wasmjit_resolver_create_cgo
#[no_mangle]
pub extern "C" fn wasmjit_onto_resolver_create() -> *mut wasmjit_resolver_t {
    let res = OntoChainResolver::new();
    let b1 = Box::new(res) as Box<dyn Resolver>;

    Box::into_raw(Box::new(b1)) as *mut wasmjit_resolver_t
}

/// Implementation of wasmjit_contruct_result_t
#[no_mangle]
pub unsafe extern "C" fn wasmjit_construct_result(
    data_buffer: *mut u8,
    data_len: u32,
    kind_t: u32,
) -> wasmjit_result_t {
    let v = std::slice::from_raw_parts(data_buffer, data_len as usize).to_vec();
    wasmjit_result_t {
        kind: kind_t,
        msg: bytes_from_vec(v),
    }
}

/// Implementation of wasmjit_service_index
#[no_mangle]
pub unsafe extern "C" fn wasmjit_service_index(vmctx: *mut wasmjit_vmctx_t) -> u64 {
    let ctx = wasmjit_vmctx_chainctx(vmctx);
    let ctx_r = convert_chain_ctx(ctx);

    ctx_r.service_index()
}

/// Implementation of wasmjit_set_calloutput
#[no_mangle]
pub unsafe extern "C" fn wasmjit_set_calloutput(
    vmctx: *mut wasmjit_vmctx_t,
    data: *mut u8,
    len: u32,
) {
    let bytes = wasmjit_bytes_new(len);
    if len != 0 {
        let buffer_g = std::slice::from_raw_parts_mut(data, len as usize);
        let buffer_i = std::slice::from_raw_parts_mut(bytes.data, bytes.len as usize);
        buffer_i.copy_from_slice(&buffer_g[..]);
    }

    let ctx = wasmjit_vmctx_chainctx(vmctx);
    wasmjit_chain_context_set_calloutput(ctx, bytes);
}

unsafe fn wasm_pointer_to_jit_slice(
    vmctx: *mut VMContext,
    data: u32,
    l: u32,
) -> Result<wasmjit_slice_t, String> {
    let mut memory = wasmjit_slice_t {
        data: ptr::null_mut(),
        len: 0,
    };
    let result = wasmjit_vmctx_memory(vmctx as *mut wasmjit_vmctx_t, &mut memory);
    check_wasmjit_result(result)?;

    let start = data as usize;
    let end = start.checked_add(l as usize);
    if end.is_none() || (end.unwrap() > memory.len as usize) {
        return Err(String::from("wasmjit: access out of bound"));
    }

    let mem = std::slice::from_raw_parts_mut(memory.data, memory.len as usize);
    Ok(wasmjit_slice_t {
        data: &mut mem[data as usize..] as *mut [u8] as *mut u8,
        len: l,
    })
}

/// Implementation of wasmjit_take_output
#[no_mangle]
unsafe fn wasmjit_take_output(instance: *mut wasmjit_instance_t) -> wasmjit_bytes_t {
    let inst = &mut *(instance as *mut Instance);
    let chain = inst.host_state();

    wasmjit_chain_context_take_output(chain as *mut ChainCtx as *mut wasmjit_chain_context_t)
}

/// Implementation of wasmjit_get_gas
#[no_mangle]
unsafe extern "C" fn wasmjit_get_gas(vmctx: *mut wasmjit_vmctx_t) -> u64 {
    let chain = wasmjit_vmctx_chainctx(vmctx);
    wasmjit_chain_context_get_gas(chain)
}

/// Implementation of wasmjit_get_gas
#[no_mangle]
unsafe extern "C" fn wasmjit_set_gas(vmctx: *mut wasmjit_vmctx_t, gas: u64) {
    let chain = wasmjit_vmctx_chainctx(vmctx);
    wasmjit_chain_context_set_gas(chain, gas);
}

/// Implementation of wasmjit_get_gas
#[no_mangle]
unsafe extern "C" fn wasmjit_get_exec_step(vmctx: *mut wasmjit_vmctx_t) -> u64 {
    let chain = wasmjit_vmctx_chainctx(vmctx);
    wasmjit_chain_context_get_exec_step(chain)
}

/// Implementation of wasmjit_get_gas
#[no_mangle]
unsafe extern "C" fn wasmjit_set_exec_step(vmctx: *mut wasmjit_vmctx_t, exec_step: u64) {
    let chain = wasmjit_vmctx_chainctx(vmctx);
    wasmjit_chain_context_set_exec_step(chain, exec_step);
}

/// Implementation of wasmjit_invoke
#[no_mangle]
pub unsafe extern "C" fn wasmjit_invoke(
    code: wasmjit_slice_t,
    chainctx: *mut wasmjit_chain_context_t,
) -> wasmjit_ret {
    let result = check_internel_panic(|| {
        let mut instance = ptr::null_mut();
        let resolver = wasmjit_onto_resolver_create();

        let res = wasmjit_instantiate(&mut instance, resolver, code);
        if res.kind != wasmjit_result_success {
            return Ok(wasmjit_ret {
                exec_step: wasmjit_chain_context_get_exec_step(chainctx),
                gas_left: wasmjit_chain_context_get_gas(chainctx),
                buffer: bytes_null(),
                res,
            });
        }

        let res = wasmjit_instance_invoke(instance, chainctx);

        // get exec_step and gas_left.
        let inst = &mut *(instance as *mut Instance);
        let chain = inst.host_state();
        let exec_step = chain.exec_step();
        let gas_left = chain.gas_left();

        let buffer = wasmjit_take_output(instance);

        wasmjit_instance_destroy(instance);
        // should destroy the instance after take output.
        Ok(wasmjit_ret {
            exec_step,
            gas_left,
            buffer, // need destroy bytes in ontology.
            res,
        })
    });

    match result {
        Ok(jit_ret) => jit_ret,
        Err(msg) => wasmjit_ret {
            exec_step: 0,
            gas_left: 0,
            buffer: bytes_null(),
            res: wasmjit_result_t {
                kind: wasmjit_result_err_internal,
                msg: bytes_from_vec(msg.into_bytes()),
            },
        },
    }
}
