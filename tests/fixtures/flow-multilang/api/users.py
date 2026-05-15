from fastapi import FastAPI
from sqlalchemy import select

app = FastAPI()


@app.get("/api/users")
def list_users(db):
    return db.execute(select(User)).all()


@app.post("/api/orders")
def create_order(body: dict, db):
    db.execute("INSERT INTO orders (data) VALUES ($1)", body)
    return {"ok": True}


class User:
    pass
