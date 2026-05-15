package example

fun routing() {
    // Ktor routes — Consumer HttpCall.
    get("/api/users/{id}")
    post("/api/users")
    delete("/api/users/{id}")
}

suspend fun callExternal() {
    // Ktor client — Producer HttpCall.
    val client = HttpClient()
    client.get("/external/api/users")
    client.post("/external/api/items")
}

fun queryUsers() {
    // Exposed — DbQuery.
    Users.select { Users.id eq 1 }
    Users.insert { it[name] = "x" }
    Posts.update({ Posts.id eq 1 }) { it[title] = "y" }
}

fun grpcCall(channel: ManagedChannel) {
    UserServiceGrpc.newBlockingStub(channel).getUser(req)
    HelloServiceGrpcKt.newCoroutineStub(channel).sayHello(req)
}

class HttpClient {
    fun get(url: String) {}
    fun post(url: String) {}
}

fun get(url: String) {}
fun post(url: String) {}
fun delete(url: String) {}

class ManagedChannel
val req: Any = Unit
object Users {
    fun select(block: (Any) -> Any) {}
    fun insert(block: (Any) -> Unit) {}
    val id = Unit
    val name = Unit
    infix fun eq(other: Any): Any = Unit
    operator fun get(idx: Any): Any = Unit
}
object Posts {
    fun update(where: () -> Any, block: (Any) -> Unit) {}
    val id = Unit
    val title = Unit
    infix fun eq(other: Any): Any = Unit
}
object UserServiceGrpc {
    fun newBlockingStub(c: Any): Stub = Stub()
    class Stub { fun getUser(req: Any): Any = req }
}
object HelloServiceGrpcKt {
    fun newCoroutineStub(c: Any): Stub = Stub()
    class Stub { suspend fun sayHello(req: Any): Any = req }
}
