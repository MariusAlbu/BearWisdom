import { connect, StringCodec } from "nats";

export async function listenForUsers(): Promise<void> {
  const nc = await connect({ servers: "nats://localhost:4222" });
  const sc = StringCodec();
  const sub = nc.subscribe("user.created");
  for await (const msg of sub) {
    const payload = JSON.parse(sc.decode(msg.data));
    console.log("received user", payload.id);
  }
}
