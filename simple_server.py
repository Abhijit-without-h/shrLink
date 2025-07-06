#!/usr/bin/env python3
"""Simple HTTP server for testing ShrLink fallback functionality"""

import os
import json
import time
import tempfile
import shutil
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse

# Directory to store uploaded files
UPLOAD_DIR = "server_files"
os.makedirs(UPLOAD_DIR, exist_ok=True)

class ShrLinkHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path == '/upload':
            self.handle_upload()
        elif self.path == '/cleanup':
            self.handle_cleanup()
        else:
            self.send_error(404, "Not found")
    
    def do_GET(self):
        if self.path.startswith('/files/'):
            self.handle_download()
        elif self.path == '/stats':
            self.handle_stats()
        else:
            self.send_error(404, "Not found")
    
    def handle_upload(self):
        try:
            # Get content length
            content_length = int(self.headers.get('Content-Length', 0))
            if content_length == 0:
                self.send_error(400, "No content")
                return
            
            # Read content type
            content_type = self.headers.get('Content-Type', '')
            if not content_type.startswith('multipart/form-data'):
                self.send_error(400, "Expected multipart/form-data")
                return
            
            # Extract boundary
            boundary = None
            for part in content_type.split(';'):
                part = part.strip()
                if part.startswith('boundary='):
                    boundary = part[9:].strip('"')
                    break
            
            if not boundary:
                self.send_error(400, "No boundary found")
                return
            
            # Read the entire request body
            body = self.rfile.read(content_length)
            
            # Parse multipart data manually (simplified)
            boundary_bytes = ('--' + boundary).encode()
            parts = body.split(boundary_bytes)
            
            filename = None
            file_data = None
            
            for part in parts:
                if b'Content-Disposition: form-data' in part and b'name="file"' in part:
                    # Extract filename
                    lines = part.split(b'\r\n')
                    for line in lines:
                        if b'filename=' in line:
                            filename_part = line.decode().split('filename=')[1].strip().strip('"')
                            if filename_part:
                                filename = filename_part
                            break
                    
                    # Extract file data (after double CRLF)
                    if b'\r\n\r\n' in part:
                        file_data = part.split(b'\r\n\r\n', 1)[1]
                        # Remove trailing CRLF if present
                        if file_data.endswith(b'\r\n'):
                            file_data = file_data[:-2]
                        break
            
            if not filename or file_data is None:
                self.send_error(400, "No file found in upload")
                return
            
            # Save the file
            file_path = os.path.join(UPLOAD_DIR, filename)
            with open(file_path, 'wb') as f:
                f.write(file_data)
            
            print(f"Uploaded file: {filename} ({len(file_data)} bytes)")
            
            # Return success response
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            response = {
                "status": "success",
                "filename": filename,
                "size": len(file_data)
            }
            self.wfile.write(json.dumps(response).encode())
            
        except Exception as e:
            print(f"Upload error: {e}")
            self.send_error(500, f"Upload failed: {e}")
    
    def handle_download(self):
        try:
            # Extract filename from path
            filename = self.path[7:]  # Remove '/files/' prefix
            file_path = os.path.join(UPLOAD_DIR, filename)
            
            if not os.path.exists(file_path):
                self.send_error(404, "File not found")
                return
            
            # Send file
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Length', str(os.path.getsize(file_path)))
            self.send_header('Content-Disposition', f'attachment; filename="{filename}"')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            
            with open(file_path, 'rb') as f:
                shutil.copyfileobj(f, self.wfile)
            
            print(f"Downloaded file: {filename}")
            
        except Exception as e:
            print(f"Download error: {e}")
            self.send_error(500, f"Download failed: {e}")
    
    def handle_cleanup(self):
        try:
            # Parse JSON request
            content_length = int(self.headers.get('Content-Length', 0))
            if content_length > 0:
                post_data = self.rfile.read(content_length)
                request_data = json.loads(post_data.decode('utf-8'))
                max_age = request_data.get('max_age_seconds', 86400)  # Default 24 hours
            else:
                max_age = 86400
            
            # Clean up old files
            deleted_count = 0
            current_time = time.time()
            
            for filename in os.listdir(UPLOAD_DIR):
                file_path = os.path.join(UPLOAD_DIR, filename)
                if os.path.isfile(file_path):
                    file_age = current_time - os.path.getmtime(file_path)
                    if file_age > max_age:
                        os.remove(file_path)
                        deleted_count += 1
                        print(f"Deleted old file: {filename}")
            
            # Return response
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            response = {"deleted_count": deleted_count}
            self.wfile.write(json.dumps(response).encode())
            
        except Exception as e:
            print(f"Cleanup error: {e}")
            self.send_error(500, f"Cleanup failed: {e}")
    
    def handle_stats(self):
        try:
            # Calculate stats
            total_files = 0
            total_bytes = 0
            
            for filename in os.listdir(UPLOAD_DIR):
                file_path = os.path.join(UPLOAD_DIR, filename)
                if os.path.isfile(file_path):
                    total_files += 1
                    total_bytes += os.path.getsize(file_path)
            
            # Return response
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Access-Control-Allow-Origin', '*')
            self.end_headers()
            response = {
                "total_files": total_files,
                "total_bytes": total_bytes
            }
            self.wfile.write(json.dumps(response).encode())
            
        except Exception as e:
            print(f"Stats error: {e}")
            self.send_error(500, f"Stats failed: {e}")
    
    def log_message(self, format, *args):
        # Custom logging
        print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] {format % args}")

def run_server():
    server = HTTPServer(('localhost', 8080), ShrLinkHandler)
    print("ShrLink HTTP server running on http://localhost:8080")
    print("Upload endpoint: POST /upload")
    print("Download endpoint: GET /files/<filename>")
    print("Stats endpoint: GET /stats")
    print("Cleanup endpoint: POST /cleanup")
    print(f"Files will be stored in: {os.path.abspath(UPLOAD_DIR)}")
    print("\nPress Ctrl+C to stop the server")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down server...")
        server.shutdown()
        server.server_close()

if __name__ == '__main__':
    run_server()
