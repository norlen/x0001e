use env_logger::init;
use x0001e::{project::Project, vm::VM};

fn main() {
    init();

    let project = Project::from_path("./examples/samples/simple/main.bc").unwrap();
    let vm = VM::new("main", &project).unwrap();

    for path in vm {
        println!("Path result: {:?}", path);
    }
}
