//! LLVM support a large number of [intrinsic functions][1] these are not implemented in bitcode.
//! Thus, these all have to be hooks that are implemented in the system.
//!
//! # Status of supported intrinsics
//!
//! ## Standard C/C++ intrinsics
//!
//! - [ ] `llvm.abs.*`
//! - [ ] `llvm.smax.*`
//! - [ ] `llvm.smin.*`
//! - [x] `llvm.umax.*`
//! - [ ] `llvm.umin.*`
//! - [x] `llvm.memcpy`
//! - [ ] `llvm.memcpy.inline`
//! - [ ] `llvm.memmove`
//! - [x] `llvm.memset`
//! - [ ] `llvm.sqrt.*`
//! - [ ] `llvm.powi.*`
//! - [ ] `llvm.sin.*`
//! - [ ] `llvm.cos.*`
//! - [ ] `llvm.pow.*`
//! - [ ] `llvm.exp.*`
//! - [ ] `llvm.exp2.*`
//! - [ ] `llvm.log.*`
//! - [ ] `llvm.log10.*`
//! - [ ] `llvm.log2.*`
//! - [ ] `llvm.fma.*`
//! - [ ] `llvm.fabs.*`
//! - [ ] `llvm.minnum.*`
//! - [ ] `llvm.maxnum.*`
//! - [ ] `llvm.minimum.*`
//! - [ ] `llvm.maximum.*`
//! - [ ] `llvm.copysign.*`
//! - [ ] `llvm.floor.*`
//! - [ ] `llvm.ceil.*`
//! - [ ] `llvm.trunc.*`
//! - [ ] `llvm.rint.*`
//! - [ ] `llvm.nearbyint.*`
//! - [ ] `llvm.round.*`
//! - [ ] `llvm.roundeven.*`
//! - [ ] `llvm.lround.*`
//! - [ ] `llvm.llround.*`
//! - [ ] `llvm.lrint.*`
//! - [ ] `llvm.llrint.*`
//!
//! ## Arithmetic with overflow intrinsics
//!
//! - [x] `llvm.sadd.with.overflow.*`
//! - [x] `llvm.uadd.with.overflow.*`
//! - [x] `llvm.ssub.with.overflow.*`
//! - [x] `llvm.usub.with.overflow.*`
//! - [x] `llvm.smul.with.overflow.*`
//! - [x] `llvm.umul.with.overflow.*`
//!
//! ## Saturation arithmetic intrinsics
//!
//! - [x] `llvm.sadd.sat.*`
//! - [x] `llvm.uadd.sat.*`
//! - [ ] `llvm.ssub.sat.*`
//! - [ ] `llvm.usub.sat.*`
//! - [ ] `llvm.sshl.sat.*`
//! - [ ] `llvm.ushl.sat.*`
//!
//! ## General intrinsics (non-exhaustive)
//!
//! - [x] `llvm.expect`
//! - [ ] `llvm.expect.with.probability`
//! - [x] `llvm.assume`
//!
//! [1]: https://llvm.org/docs/LangRef.html#intrinsic-functions
use llvm_ir::Type;
use log::trace;
use radix_trie::Trie;
use std::collections::HashMap;

use crate::{
    common::{binop, get_u64_solution_from_operand},
    hooks::{FnInfo, Hook},
    memory::BITS_IN_BYTE,
    vm::{Result, ReturnValue, VM},
    VMError, BV,
};

/// Check if the given name is an LLVM intrinsic.
///
/// Currently it checks that the name starts with `llvm.` which seems like a good approximation.
pub(super) fn is_intrinsic(name: &str) -> bool {
    name.starts_with("llvm.")
}

/// Intrinsic hook storage.
///
/// Keeps track of intrinsics that have only one version such as `llvm.va_start` and those with
/// multiple versions such as `llvm.abs.*` which is valid for multiple bit lengths.
///
/// Internally fixed length name intrinsics use a `[std::collections::HashMap]` so all lookups are
/// constant time. Variable intrinsic names use a `[radix_trie::Trie]` so lookups are linear time of
/// the retrieved name.
pub(super) struct Intrinsics {
    /// Fixed length intrinsic values, e.g. `llvm.va_start`.
    fixed: HashMap<String, Hook>,

    /// Intrinsics with a suffix such as `llvm.abs.*`.
    ///
    /// Note that the field does not care what the suffix is, it only finds the closest ancestor
    /// (if any).
    variable: Trie<String, Hook>,
}

impl Intrinsics {
    /// Creates a new intrinsic hook storage with all the default intrinsics enabled.
    pub(super) fn new_with_defaults() -> Self {
        let mut s = Self {
            fixed: HashMap::new(),
            variable: Trie::new(),
        };

        // Add fixed intrinsics.
        s.add_fixed("llvm.assume", llvm_assume);

        // Add variable intrinsics.
        s.add_variable("llvm.memcpy.", llvm_memcpy);
        s.add_variable("llvm.memset.", llvm_memset);
        s.add_variable("llvm.umax.", llvm_umax);

        s.add_variable("llvm.sadd.with.overflow.", llvm_sadd_with_overflow);
        s.add_variable("llvm.uadd.with.overflow.", llvm_uadd_with_overflow);
        s.add_variable("llvm.ssub.with.overflow.", llvm_ssub_with_overflow);
        s.add_variable("llvm.usub.with.overflow.", llvm_usub_with_overflow);
        s.add_variable("llvm.smul.with.overflow.", llvm_smul_with_overflow);
        s.add_variable("llvm.umul.with.overflow.", llvm_umul_with_overflow);

        s.add_variable("llvm.sadd.sat.", llvm_sadd_sat);
        s.add_variable("llvm.uadd.sat.", llvm_uadd_sat);

        s.add_variable("llvm.expect.", llvm_expect);

        // Temporary.
        s.add_variable("llvm.dbg", noop);
        s.add_variable("llvm.lifetime", noop);
        s.add_variable("llvm.experimental", noop);

        s
    }

    /// Add a fixed length intrinsic.
    fn add_fixed(&mut self, name: impl Into<String>, hook: Hook) {
        self.fixed.insert(name.into(), hook);
    }

    /// Add a variable length intrinsic, e.g. if they support any kind of bit width such as
    /// `llvm.abs.*`.
    ///
    /// Note that it matches for the entire prefix, so to add `llvm.abs.*` the name should only be
    /// `llvm.abs.`
    fn add_variable(&mut self, name: impl Into<String>, hook: Hook) {
        self.variable.insert(name.into(), hook);
    }

    /// Returns a reference to the hook of the given name. If the hook cannot be found `None` is
    /// returned.
    ///
    /// It first checks the fixed length names, and if such a name cannot be found it checks the
    /// variable length names.
    pub(super) fn get(&self, name: &str) -> Option<&Hook> {
        self.fixed
            .get(name)
            .or_else(|| self.variable.get_ancestor_value(name))
    }
}

pub fn noop(_vm: &mut VM<'_>, _f: FnInfo) -> Result<ReturnValue> {
    Ok(ReturnValue::Void)
}

// -------------------------------------------------------------------------------------------------
// Standard C/C++ intrinsics
// -------------------------------------------------------------------------------------------------

/// Copy a block of memory from the source to the destination.
pub fn llvm_memcpy(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    // Arguments:
    // 1. Pointer to destination.
    // 2. Pointer to source.
    // 3. Integer, number of bytes to copy.
    // 4. Bool, indicates volatile access.
    // The first two arguments can have an optional alignment.
    //
    // The source and destination must either be equal or non-overlapping. If the length is not a
    // well-defined value the behavior is undefined. Pointers to source and destination should be
    // well-defined is the length is not zero.
    //
    // TODO: What is a `well-defined` value?
    // TODO: Check the isvolatile and the details of volatile operations.
    assert_eq!(f.arguments.len(), 4);
    trace!("llvm_memcpy");

    let (dst, _) = &f.arguments[0];
    let (src, _) = &f.arguments[1];
    let (size, _) = &f.arguments[2];

    let dst = vm.state.get_var(dst)?;
    let src = vm.state.get_var(src)?;

    let size = get_u64_solution_from_operand(&vm.state, size)?;
    let size = size as u32 * BITS_IN_BYTE;

    let value = vm.state.mem.borrow_mut().read(&src, size)?;
    vm.state.mem.borrow_mut().write(&dst, value)?;

    Ok(ReturnValue::Void)
}

pub fn llvm_memset(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    // Arguments:
    // 1. Pointer to address to fill.
    // 2. Byte to to fill with.
    // 3. Number of bytes to fill.
    // 4. Indicates volatile access.
    assert_eq!(f.arguments.len(), 4);
    trace!("llvm_memset");

    let (dst, _) = &f.arguments[0];
    let (value, _) = &f.arguments[1];
    let (size, _) = &f.arguments[2];

    let dst = vm.state.get_var(dst)?;
    let value = vm.state.get_var(value)?;
    assert_eq!(value.len(), BITS_IN_BYTE);

    let size = get_u64_solution_from_operand(&vm.state, size)?;

    for byte in 0..size {
        let offset = vm.solver.bv_from_u64(byte, vm.project.ptr_size);
        let addr = dst.add(&offset);

        vm.state.mem.borrow_mut().write(&addr, value.clone())?;
    }

    Ok(ReturnValue::Void)
}

pub fn llvm_umax(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    assert_eq!(f.arguments.len(), 2);
    let lhs = &f.arguments[0].0;
    let rhs = &f.arguments[1].0;

    let result = binop(&vm.state, lhs, rhs, |lhs, rhs| {
        let condition = lhs.ugt(rhs);
        condition.ite(lhs, rhs)
    })?;

    Ok(ReturnValue::Value(result))
}

// -------------------------------------------------------------------------------------------------
// Arithmetic with overflow intrinsics
// -------------------------------------------------------------------------------------------------

/// All the binary operations that check for overflow.
enum BinaryOpOverflow {
    SAdd,
    UAdd,
    SSub,
    USub,
    SMul,
    UMul,
}

/// Binary operations that indicate whether an overflow occurred or not.
fn binary_op_overflow(vm: &mut VM<'_>, f: FnInfo, op: BinaryOpOverflow) -> Result<ReturnValue> {
    assert_eq!(f.arguments.len(), 2);

    let (lhs, _) = f.arguments.get(0).unwrap();
    let (rhs, _) = f.arguments.get(1).unwrap();

    let lhs_ty = vm.state.type_of(lhs);
    let rhs_ty = vm.state.type_of(rhs);
    let lhs = vm.state.get_var(lhs)?;
    let rhs = vm.state.get_var(rhs)?;

    let operation = |lhs: BV, rhs: BV| {
        let (result, overflow) = match op {
            BinaryOpOverflow::SAdd => (lhs.add(&rhs), lhs.saddo(&rhs)),
            BinaryOpOverflow::UAdd => (lhs.add(&rhs), lhs.uaddo(&rhs)),
            BinaryOpOverflow::SSub => (lhs.sub(&rhs), lhs.ssubo(&rhs)),
            BinaryOpOverflow::USub => (lhs.sub(&rhs), lhs.usubo(&rhs)),
            BinaryOpOverflow::SMul => (lhs.mul(&rhs), lhs.smulo(&rhs)),
            BinaryOpOverflow::UMul => (lhs.mul(&rhs), lhs.umulo(&rhs)),
        };
        assert_eq!(overflow.len(), 1);
        trace!("result: {result:?}, overflow: {overflow:?}");

        (result, overflow)
    };

    // This has to be special cased a bit compared to regular binary operations.
    //
    // The result type is a struct so {result, overflow} and for vectors this means {<iX res>, <i1>}
    // so the results and overflows have to be appended separately until the final return. Which the
    // regular `binop` does not handle.
    use Type::*;
    let result = match (lhs_ty.as_ref(), rhs_ty.as_ref()) {
        // For integers just perform the operation.
        (IntegerType { .. }, IntegerType { .. }) => {
            let (result, overflow) = operation(lhs, rhs);
            Ok(overflow.concat(&result))
        }

        // For vectors each operation has to be done independently, and the return should be in
        // the format of {results, overflows}.
        (
            VectorType {
                element_type: lhs_inner_ty,
                num_elements: n,
                scalable: false,
            },
            VectorType {
                element_type: rhs_inner_ty,
                num_elements: m,
                scalable: false,
            },
        ) => {
            assert_eq!(lhs_inner_ty, rhs_inner_ty);
            assert_eq!(
                vm.state.project.bit_size(lhs_inner_ty),
                vm.state.project.bit_size(rhs_inner_ty)
            );
            assert_eq!(*n, *m);

            let bits = vm.state.project.bit_size(lhs_inner_ty)?;
            let num_elements = *n as u32;

            // Perform the operation per element and concatenate the result.
            let (results, overflows) = (0..num_elements)
                .map(|i| {
                    let low = i * bits;
                    let high = (i + 1) * bits - 1;
                    let lhs = lhs.slice(low, high);
                    let rhs = rhs.slice(low, high);
                    operation(lhs, rhs)
                })
                .reduce(|(res_acc, overflow_acc), (res, overflow)| {
                    (res.concat(&res_acc), overflow.concat(&overflow_acc))
                })
                .ok_or(VMError::MalformedInstruction)?;

            trace!("results: {results:?}, overflows: {overflows:?}");
            Ok(overflows.concat(&results))
        }

        // TODO: Check out scalable vectors.
        (VectorType { .. }, VectorType { .. }) => {
            todo!()
        }

        // These types should not appear in a binary operation.
        _ => Err(VMError::MalformedInstruction),
    }?;

    Ok(ReturnValue::Value(result))
}

/// Signed addition on any bit width, performs a signed addition and indicates whether an overflow
/// occurred.
pub fn llvm_sadd_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::SAdd)
}

/// Unsigned addition on any bit width, performs an unsigned addition and indicates whether an
/// overflow occurred.
pub fn llvm_uadd_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::UAdd)
}

/// Signed subtraction on any bit width, performs a signed subtraction and indicates whether an
/// overflow occurred.
pub fn llvm_ssub_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::SSub)
}

/// Unsigned subtraction on any bit width, performs an unsigned subtraction and indicates whether an
/// overflow occurred.
pub fn llvm_usub_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::USub)
}

/// Signed multiplication on any bit width, performs a signed multiplication and indicates whether
/// an overflow occurred.
pub fn llvm_smul_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::SMul)
}

/// Unsigned multiplication on any bit width, performs an unsigned multiplication and indicates
/// whether an overflow occurred.
pub fn llvm_umul_with_overflow(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_overflow(vm, f, BinaryOpOverflow::UMul)
}

// -------------------------------------------------------------------------------------------------
// Saturation arithmetic intrinsics
// -------------------------------------------------------------------------------------------------

enum BinaryOpSaturate {
    SAdd,
    UAdd,
}

fn binary_op_saturate(vm: &mut VM<'_>, f: FnInfo, op: BinaryOpSaturate) -> Result<ReturnValue> {
    assert_eq!(f.arguments.len(), 2);

    let (lhs, _) = f.arguments.get(0).unwrap();
    let (rhs, _) = f.arguments.get(1).unwrap();

    let result = binop(&vm.state, lhs, rhs, |lhs, rhs| match op {
        BinaryOpSaturate::UAdd => lhs.uadds(&rhs),
        BinaryOpSaturate::SAdd => lhs.sadds(&rhs),
    })?;

    Ok(ReturnValue::Value(result))
}

pub fn llvm_uadd_sat(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_saturate(vm, f, BinaryOpSaturate::UAdd)
}
pub fn llvm_sadd_sat(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    binary_op_saturate(vm, f, BinaryOpSaturate::SAdd)
}

// -------------------------------------------------------------------------------------------------
// General intrinsics
// -------------------------------------------------------------------------------------------------

pub fn llvm_expect(vm: &mut VM<'_>, f: FnInfo) -> Result<ReturnValue> {
    assert_eq!(f.arguments.len(), 2);
    let (a0, _) = f.arguments.get(0).unwrap();
    let val = vm.state.get_var(a0).unwrap();

    Ok(ReturnValue::Value(val))
}

pub fn llvm_assume(vm: &mut VM<'_>, info: FnInfo) -> Result<ReturnValue> {
    assert_eq!(info.arguments.len(), 1);

    let (condition, _) = &info.arguments[0];
    let condition = vm.state.get_var(condition)?;
    vm.state.solver.assert(&condition);

    Ok(ReturnValue::Void)
}

#[cfg(test)]
mod tests {
    use crate::{Project, ReturnValue, Solutions, VMError, VM};

    fn run(fn_name: &str) -> Vec<Result<Option<i64>, VMError>> {
        let path = format!("./tests/unit_tests/intrinsics.bc");
        let project = Project::from_path(&path).expect("Failed to created proejct");
        let mut vm = VM::new(fn_name, &project).expect("Failed to create VM");

        let mut path_results = Vec::new();
        while let Some(path_result) = vm.run() {
            let path_result = match path_result {
                Ok(value) => match value {
                    ReturnValue::Value(value) => {
                        let sol = vm.solver.get_solutions_for_bv(&value, 1).unwrap();
                        match sol {
                            Solutions::None => panic!("No solutions"),
                            Solutions::Exactly(s) => Ok(Some(s[0].as_u64().unwrap() as i64)),
                            Solutions::AtLeast(_) => panic!("More than one solution"),
                        }
                    }
                    ReturnValue::Void => Ok(None),
                },
                Err(e) => Err(e),
            };
            path_results.push(path_result);
        }

        println!("{path_results:x?}");
        path_results
    }

    #[test]
    fn test_memcpy() {
        let res = run("test_memcpy");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x6543fe671234abcd)));
    }

    #[test]
    fn test_memset() {
        let res = run("test_memset");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(-6076574517859464245i64))); // 0xababababcbcbcbcb
    }

    #[test]
    fn test_umax() {
        let res = run("test_umax");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0xbcef)));
    }

    #[test]
    fn test_umax_vec() {
        let res = run("test_umax_vec");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x000043210000bcef)));
    }

    #[test]
    fn test_sadd_sat0() {
        let res = run("test_sadd_sat0");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(3)));
    }

    #[test]
    fn test_sadd_sat1() {
        let res = run("test_sadd_sat1");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(7)));
    }

    #[test]
    fn test_sadd_sat2() {
        let res = run("test_sadd_sat2");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(-2)));
    }

    #[test]
    fn test_sadd_sat3() {
        let res = run("test_sadd_sat3");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(-8)));
    }

    #[test]
    fn test_uadd_sat0() {
        let res = run("test_uadd_sat0");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(3)));
    }

    #[test]
    fn test_uadd_sat1() {
        let res = run("test_uadd_sat1");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(11)));
    }

    #[test]
    fn test_uadd_sat2() {
        let res = run("test_uadd_sat2");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(15)));
    }

    #[test]
    fn test_uadd_sat_vec() {
        let res = run("test_uadd_sat_vec");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0xfb3)));
    }

    #[test]
    fn test_sadd_with_overflow0() {
        let res = run("test_sadd_with_overflow0");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x0182)));
    }

    #[test]
    fn test_sadd_with_overflow1() {
        let res = run("test_sadd_with_overflow1");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x6e)));
    }

    #[test]
    fn test_sadd_with_overflow_vec() {
        let res = run("test_sadd_with_overflow_vec");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x5b07e6e82)));
    }

    #[test]
    fn test_uadd_with_overflow0() {
        let res = run("test_uadd_with_overflow0");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x0104)));
    }

    #[test]
    fn test_uadd_with_overflow1() {
        let res = run("test_uadd_with_overflow1");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(0x00fa)));
    }

    #[test]
    fn test_expect() {
        let res = run("test_expect");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(100)));
    }

    #[test]
    fn test_assume() {
        let res = run("test_assume");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], Ok(Some(5)));
    }
}
