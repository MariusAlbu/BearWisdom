package example;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RestController;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.web.client.RestTemplate;
import retrofit2.http.GET;
import retrofit2.http.POST;
import io.grpc.ManagedChannel;

@RestController
public class App {

    JdbcTemplate jdbcTemplate;
    RestTemplate restTemplate;

    @GetMapping("/api/users/{id}")
    public String getUser(String id) { return id; }

    @PostMapping("/api/users")
    public String createUser(String body) { return body; }

    public void jdbcWork() {
        jdbcTemplate.query("SELECT id, name FROM users WHERE active = ?", new Object[]{1}, (rs, n) -> rs.getString(1));
        jdbcTemplate.update("UPDATE accounts SET balance = ? WHERE id = ?", 100, 1);
        jdbcTemplate.queryForList("SELECT * FROM items");
    }

    public void httpClient() {
        restTemplate.getForObject("/external/api/v1/data", String.class);
        restTemplate.postForObject("/external/api/v1/items", "body", String.class);
    }

    public void grpcCall(ManagedChannel channel) {
        UserServiceGrpc.newBlockingStub(channel).getUser(GetUserRequest.newBuilder().setId("1").build());
        HelloServiceGrpc.newBlockingStub(channel).sayHello(HelloRequest.newBuilder().build());
    }
}

interface UserApi {
    @GET("/users/{id}")
    String getUser(String id);

    @POST("/users")
    String createUser(String body);
}

class UserServiceGrpc {
    public static Stub newBlockingStub(ManagedChannel c) { return new Stub(); }
    static class Stub {
        public Object getUser(Object req) { return req; }
    }
}

class HelloServiceGrpc {
    public static Stub newBlockingStub(ManagedChannel c) { return new Stub(); }
    static class Stub {
        public Object sayHello(Object req) { return req; }
    }
}

class GetUserRequest {
    public static Builder newBuilder() { return new Builder(); }
    static class Builder {
        public Builder setId(String id) { return this; }
        public GetUserRequest build() { return new GetUserRequest(); }
    }
}
class HelloRequest {
    public static Builder newBuilder() { return new Builder(); }
    static class Builder {
        public HelloRequest build() { return new HelloRequest(); }
    }
}
