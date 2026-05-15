import { Server } from "@grpc/grpc-js";

declare const UserService: any;

export const server = new Server();
server.addService(UserService, {
  getUser: async (req: { id: string }) => ({ name: "alice" }),
  listUsers: async () => ({ users: [] }),
});
