use x0001e::{
    memory::{MemoryError, CHECK_OUT_OF_BOUNDS},
    vm::VMError,
};

mod test_runner;

use test_runner::{run, ConcreteValue};

fn as_u64(v: &Option<ConcreteValue>) -> Option<u64> {
    if let Some(v) = v {
        if let ConcreteValue::Value { value, .. } = v {
            Some(*value)
        } else {
            None
        }
    } else {
        None
    }
}

// Check that simple instructions work.
#[test]
fn simple() {
    let res = run("tests/samples/simple.bc", "main").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(0)));
}

// Check that simple if statements work.
//
// The path is hardcoded so only a single path should be taken.
#[test]
fn ifs() {
    let res = run("tests/samples/ifs.bc", "main").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(1)));
}

// Check that if statements work.
//
// The input here is symbolic so both paths should be taken.
#[test]
fn multiple_paths() {
    let res = run("tests/samples/multiple_paths.bc", "foo").unwrap();
    assert_eq!(res.len(), 2, "expected 2 paths");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(1)));
    assert_eq!(res[1].result.as_ref().map(as_u64), Ok(Some(2)));
}

// Check that function calls work.
//
// This also has backtracking so it ensures it correctly handles re-entry into an inner function.
#[test]
fn call() {
    let res = run("tests/samples/call.bc", "bar").unwrap();
    assert_eq!(res.len(), 2, "expected 2 paths");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(10)));
    assert_eq!(res[1].result.as_ref().map(as_u64), Ok(Some(1)));
}

// Test that it can handle traits (global variable vtables).
#[test]
fn traits() {
    let res = run("tests/samples/traits.bc", "traits::foo").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(10)));
}

// Test that it handles match expressions.
#[test]
fn match_works() {
    let res = run("tests/samples/match.bc", "match::main").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result, Ok(None));
}

// Check that it can handle global references to functions.
//
// This will call one of two functions depending on a bool.
#[test]
fn fn_refs() {
    let res = run("tests/samples/fn_refs.bc", "fn_refs::foo").unwrap();
    assert_eq!(res.len(), 2, "expected 2 paths");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(10)));
    assert_eq!(res[1].result.as_ref().map(as_u64), Ok(Some(11)));
}

// Check that array indexing works.
//
// For typical checked array gets it will return panic on out of bounds.
#[test]
fn array_index() {
    let res = run("tests/samples/array_index.bc", "array_index::get").unwrap();
    assert_eq!(res.len(), 2, "expected 2 paths");
    assert!(res[0].result.is_ok());
    assert!(res[0].inputs[0].as_u64() <= 3);

    assert_eq!(res[1].result, Err(VMError::Abort(-1)));
    assert!(res[1].inputs[0].as_u64() > 3);
}

// Check that array indexing works.
#[test]
fn array_index_works() {
    let res = run(
        "tests/samples/array_index.bc",
        "array_index::indexing_works",
    )
    .unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(3)));
}

// Check that array indexing works even when using the unsafe `get_unchecked`
#[test]
fn array_index_get_unchecked() {
    if CHECK_OUT_OF_BOUNDS {
        let res = run("tests/samples/array_index.bc", "array_index::get_unchecked").unwrap();
        assert_eq!(res.len(), 1, "expected 1 path");
        assert_eq!(
            res[0].result,
            Err(VMError::MemoryError(MemoryError::OutOfBounds))
        );
    }
}

// Check that basic loops work
#[test]
fn loops_work() {
    let res = run("tests/samples/loop.bc", "loop::simple_loop_works").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(0xbc)));
}

//
#[test]
fn structs_work() {
    let res = run("tests/samples/structs.bc", "structs::foo").unwrap();
    assert_eq!(res.len(), 1, "expected 1 path");
    assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(15)));
}

// #[test]
// fn structs_output_is_good() {
//     let res = run("tests/samples/structs.bc", "structs::foobar").unwrap();
//     assert_eq!(res.len(), 1, "expected 1 path");
//     // assert_eq!(res[0].result.as_ref().map(as_u64), Ok(Some(15)));
// }
