<script lang="ts">
  import axios from 'axios';
  async function loadUsers() {
    return await axios.get('/api/users');
  }
</script>

<h1>Hello</h1>
