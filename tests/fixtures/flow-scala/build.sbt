name := "flow-scala"
version := "0.1.0"
scalaVersion := "3.3.0"
libraryDependencies ++= Seq(
  "com.typesafe.akka" %% "akka-http" % "10.5.0",
  "com.softwaremill.sttp.client3" %% "core" % "3.9.0",
  "com.typesafe.slick" %% "slick" % "3.5.0",
  "io.grpc" % "grpc-stub" % "1.60.0"
)
