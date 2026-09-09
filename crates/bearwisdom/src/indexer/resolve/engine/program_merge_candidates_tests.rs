use super::*;

#[test]
fn staging_an_unknown_program_does_not_manufacture_a_context() {
    let graph = Graph::default();
    let program = ProgramId {
        snapshot: 123,
        index: 0,
    };
    assert!(graph
        .staged(program, &Default::default())
        .nominal_context(program)
        .is_none());
}
