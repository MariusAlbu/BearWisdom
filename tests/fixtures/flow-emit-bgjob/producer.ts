import { Queue } from "bullmq";

const emailQueue = new Queue("email");

export async function enqueueWelcome(userId: number): Promise<void> {
  await emailQueue.add("send-welcome", { userId });
}
