import requests
import httpx
import urllib.request
from fastapi import FastAPI
from sqlalchemy import select, insert
from django.urls import path, re_path

app = FastAPI()


# FastAPI Consumer.
@app.get("/api/users/{user_id}")
def get_user(user_id: int):
    return {"id": user_id}


@app.post("/api/users")
def create_user(body: dict):
    return body


# Django routes Consumer.
urlpatterns = [
    path("dj/users/", get_user),
    re_path(r"^dj/items/(?P<id>\d+)/$", get_user),
]


# requests / httpx / urllib Producer.
def call_external():
    requests.get("/external/users")
    requests.post("/external/users", json={})
    client = httpx.Client()
    client.get("/api/v1/data")
    urllib.request.urlopen("https://api.example.com/x")


# SQLAlchemy 2.x DbQuery.
def query_sqlalchemy(db):
    db.execute(select(User))
    db.execute(insert(User).values(name="x"))


# Raw cursor DbQuery.
def query_raw(cursor):
    cursor.execute("SELECT id, name FROM users WHERE active = 1")
    cursor.execute("UPDATE accounts SET balance = 1 WHERE id = 2")


# gRPC Producer.
def grpc_call(channel):
    UserServiceStub(channel).GetUser(GetUserRequest(id="1"))
    HelloServiceStub(channel).SayHello(HelloRequest())


class User:
    pass


class UserServiceStub:
    def __init__(self, channel): pass
    def GetUser(self, req): pass


class HelloServiceStub:
    def __init__(self, channel): pass
    def SayHello(self, req): pass
