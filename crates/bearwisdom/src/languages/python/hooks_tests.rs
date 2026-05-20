use super::PythonHooks;

#[test]
fn static_instance_send_sync() {
    fn require<T: Send + Sync + ?Sized>() {}
    require::<PythonHooks>();
}
