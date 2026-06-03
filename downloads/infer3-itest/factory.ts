export class User {
  name: string = "";
  greet(): string {
    return "hi " + this.name;
  }
}

// Un-annotated factory: its return type must be INFERRED as User from the body.
export function makeUser() {
  return new User();
}
