// Private-LAN QA only; never forwards to the daily Core or logs credentials.
import http from 'node:http';
import net from 'node:net';
const bind = process.env.RMUSIC_QA_LAN_BIND;
if (!/^10\.\d+\.\d+\.\d+$|^192\.168\.\d+\.\d+$|^172\.(1[6-9]|2\d|3[01])\.\d+\.\d+$/.test(bind || '')) throw Error('Private LAN bind required');
const server = http.createServer((request, response) => {
  const upstream = http.request({hostname:'127.0.0.1',port:18874,path:request.url,method:request.method,headers:request.headers}, result => {
    response.writeHead(result.statusCode, result.headers);
    result.pipe(response);
  });
  upstream.on('error', () => { if (!response.headersSent) response.writeHead(502);response.end(); });
  response.on('close', () => upstream.destroy());
  request.on('aborted', () => upstream.destroy());
  request.pipe(upstream);
});
server.on('upgrade', (request, socket, head) => {
  const upstream = net.connect(18874, '127.0.0.1', () => {
    upstream.write(`${request.method} ${request.url} HTTP/${request.httpVersion}\r\n`);
    for(let i=0;i<request.rawHeaders.length;i+=2) upstream.write(`${request.rawHeaders[i]}: ${request.rawHeaders[i+1]}\r\n`);
    upstream.write('\r\n');if(head.length)upstream.write(head);
    socket.pipe(upstream);upstream.pipe(socket);
  });
  upstream.on('error',()=>socket.destroy());socket.on('error',()=>upstream.destroy());
  socket.on('close',()=>upstream.destroy());upstream.on('close',()=>socket.destroy());
});
server.listen(18876,bind,()=>console.log('Isolated QA HTTP/control proxy on private LAN port18876'));
