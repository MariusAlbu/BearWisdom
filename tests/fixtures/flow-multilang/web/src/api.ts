import axios from 'axios';

export async function listUsers() {
  return await axios.get('/api/users');
}

export async function getItem(id: string) {
  return await axios.get(`/api/items/${id}`);
}

export async function createOrder(data: unknown) {
  return await axios.post('/api/orders', data);
}
