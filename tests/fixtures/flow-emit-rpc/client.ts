import { Client } from "@grpc/grpc-js";

declare const client: any;

export async function fetchUser(id: string): Promise<void> {
  client.userService.getUser({ id });
}

void Client;
