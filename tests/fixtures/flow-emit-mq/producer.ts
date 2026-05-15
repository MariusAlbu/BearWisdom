import { connect, StringCodec } from "nats";

export async function publishUserCreated(id: number): Promise<void> {
  const nc = await connect({ servers: "nats://localhost:4222" });
  const sc = StringCodec();
  nc.publish("user.created", sc.encode(JSON.stringify({ id })));
  await nc.drain();
}
