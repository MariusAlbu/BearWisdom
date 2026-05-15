<script setup lang="ts">
import axios from 'axios';

async function loadUsers() {
  return await axios.get('/api/users');
}
</script>

<template>
  <div>Hello</div>
</template>
