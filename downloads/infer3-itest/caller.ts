import { makeUser } from "./factory";

// `u` has no annotation; its type comes from makeUser()'s INFERRED return.
// `u.greet()` resolves only if INFER-3 inferred makeUser(): User and INFER-2
// propagated it so the forward-inference cache types `u` as User.
const u = makeUser();
u.greet();
