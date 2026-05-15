import { createTransport } from "nodemailer";

const transport = createTransport({ host: "smtp.example.com" });

export async function welcomeUser(email: string): Promise<void> {
  await transport.sendMail({
    to: email,
    template: "welcome",
    subject: "Welcome!",
  });
}
