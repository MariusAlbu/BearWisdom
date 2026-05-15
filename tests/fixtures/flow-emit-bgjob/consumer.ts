import { Worker } from "bullmq";

export const emailWorker = new Worker("email", async (job) => {
  if (job.name === "send-welcome") {
    console.log("welcoming", job.data.userId);
  }
});
