package example

object App {
  def routes = {
    path("api/users") { complete("ok") }
    pathPrefix("api/items") { complete("ok") }
  }

  def callOut(client: Client) = {
    client.get("/external/api/users")
    client.post("/external/api/items")
  }

  def querySlick = {
    Users.filter(_.id === 1).result
    Posts.insert
    Comments.delete
  }

  def grpcCall = {
    UserServiceGrpc.stub(channel).getUser(req)
    HelloServiceGrpc.blockingStub(channel).sayHello(req)
  }

  // stubs
  def path(s: String)(block: => Any): Any = ()
  def pathPrefix(s: String)(block: => Any): Any = ()
  def complete(s: String): Any = ()
  val channel: Any = ()
  val req: Any = ()
}
class Client {
  def get(url: String): Any = ()
  def post(url: String): Any = ()
}
object Users { def filter(f: Any => Any): Builder = Builder(); }
object Posts { def insert: Any = () }
object Comments { def delete: Any = () }
case class Builder() { def result: Any = (); def === (other: Any): Any = () }
object UserServiceGrpc {
  def stub(c: Any): Stub = Stub()
  case class Stub() { def getUser(req: Any): Any = req }
}
object HelloServiceGrpc {
  def blockingStub(c: Any): Stub = Stub()
  case class Stub() { def sayHello(req: Any): Any = req }
}
