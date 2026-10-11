// Benchmark server for Express 5.
//
// Run with `node bench/express_app.js`. Serves the same four endpoints
// as examples/bench_server.rs on port 3002.

const express = require('express');
const app = express();

// A five-deep middleware chain, as the /chain endpoint's setup.
for (let i = 0; i < 5; i++) {
  app.use((req, res, next) => {
    res.set('x-chain', '1');
    next();
  });
}

app.get('/hello', (req, res) => res.send('Hello, World!'));
app.get('/json', (req, res) => res.json({ message: 'Hello, World!' }));
app.get('/user/:id', (req, res) => res.send('user ' + req.params.id));
app.get('/chain', (req, res) => res.send('chained'));

app.listen(3002, () => console.log('express on 3002'));
