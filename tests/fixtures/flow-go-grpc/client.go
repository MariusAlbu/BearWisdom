package main

import (
	"context"
	"database/sql"
	"fmt"
	"net/http"

	"google.golang.org/grpc"
	userpb "example.com/flow-go-grpc/proto/userpb"
)

func main() {
	conn, _ := grpc.Dial("localhost:50051", grpc.WithInsecure())
	defer conn.Close()
	client := userpb.NewUserServiceClient(conn)

	resp, err := client.GetUser(context.Background(), &userpb.GetUserRequest{Id: "1"})
	if err != nil {
		fmt.Println(err)
		return
	}
	fmt.Println(resp.Name)

	http.Get("/api/users")

	var db *sql.DB
	db.Query("SELECT id, name FROM users WHERE active = $1", true)
	db.Exec("UPDATE accounts SET balance = $1 WHERE id = $2", 100, 1)
}
