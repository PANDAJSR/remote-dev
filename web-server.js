const http = require('http');
const fs = require('fs');
const path = require('path');
const os = require('os');

const PORT = 8080;
const DIRECTORY = 'D:\\Code\\remote-dev';

// Get all IP addresses
function getIPAddresses() {
  const interfaces = os.networkInterfaces();
  const addresses = [];
  
  for (const name of Object.keys(interfaces)) {
    for (const iface of interfaces[name]) {
      if (iface.family === 'IPv4' && !iface.internal) {
        addresses.push({ name: name, address: iface.address });
      }
    }
  }
  return addresses;
}

const mimeTypes = {
  '.html': 'text/html',
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.txt': 'text/plain'
};

const server = http.createServer((req, res) => {
  let filePath = path.join(DIRECTORY, req.url === '/' ? 'index.html' : req.url);
  
  // Security: prevent directory traversal
  if (!filePath.startsWith(DIRECTORY)) {
    res.writeHead(403);
    res.end('Forbidden');
    return;
  }
  
  const ext = path.extname(filePath).toLowerCase();
  const contentType = mimeTypes[ext] || 'application/octet-stream';
  
  fs.readFile(filePath, (err, content) => {
    if (err) {
      if (err.code === 'ENOENT') {
        // File not found, show directory listing
        fs.readdir(DIRECTORY, (err, files) => {
          if (err) {
            res.writeHead(500);
            res.end('Server Error');
            return;
          }
          
          let html = `<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <title>Directory Listing - ${req.url}</title>
  <style>
    body { font-family: Arial, sans-serif; margin: 40px; background: #f5f5f5; }
    h1 { color: #333; }
    ul { list-style: none; padding: 0; }
    li { margin: 10px 0; }
    a { color: #0366d6; text-decoration: none; font-size: 16px; }
    a:hover { text-decoration: underline; }
    .folder { font-weight: bold; }
    .file { color: #666; }
  </style>
</head>
<body>
  <h1>📁 Directory Listing</h1>
  <p>Path: <code>${DIRECTORY}</code></p>
  <ul>`;
          
          files.forEach(file => {
            const isDir = fs.statSync(path.join(DIRECTORY, file)).isDirectory();
            const icon = isDir ? '📁' : '📄';
            const className = isDir ? 'folder' : 'file';
            html += `<li>${icon} <a href="${file}" class="${className}">${file}</a></li>`;
          });
          
          html += `</ul>
  <hr>
  <p><small>Server running on port ${PORT}</small></p>
</body>
</html>`;
          
          res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
          res.end(html);
        });
      } else {
        res.writeHead(500);
        res.end('Server Error: ' + err.code);
      }
    } else {
      res.writeHead(200, { 'Content-Type': contentType });
      res.end(content);
    }
  });
});

server.listen(PORT, '0.0.0.0', () => {
  console.log(`🚀 Server running at:`);
  console.log(`   Local: http://localhost:${PORT}`);
  
  const ips = getIPAddresses();
  if (ips.length > 0) {
    console.log(`\n📡 Network interfaces:`);
    ips.forEach(ip => {
      console.log(`   ${ip.name}: http://${ip.address}:${PORT}`);
    });
  }
  
  console.log(`\n📂 Serving directory: ${DIRECTORY}`);
  console.log(`\nPress Ctrl+C to stop`);
});

// Handle graceful shutdown
process.on('SIGINT', () => {
  console.log('\n\n👋 Shutting down server...');
  server.close(() => {
    process.exit(0);
  });
});
