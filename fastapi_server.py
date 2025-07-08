#!/usr/bin/env python3
"""FastAPI server for ShrLink with improved features and testing endpoints"""

import os
import json
import time
import uuid
import tempfile
import shutil
from pathlib import Path
from typing import Optional

from fastapi import FastAPI, File, UploadFile, HTTPException, status
from fastapi.responses import FileResponse, HTMLResponse
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
import uvicorn

# Directory to store uploaded files
UPLOAD_DIR = Path("server_files")
UPLOAD_DIR.mkdir(exist_ok=True)

app = FastAPI(
    title="ShrLink Server",
    description="HTTP fallback server for ShrLink P2P file sharing",
    version="1.0.0"
)

# Enable CORS for web UI
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

class CleanupRequest(BaseModel):
    max_age_seconds: int = 86400  # 24 hours default

class FileUploadResponse(BaseModel):
    status: str
    filename: str
    size: int
    download_url: str

class StatsResponse(BaseModel):
    total_files: int
    total_bytes: int

class CleanupResponse(BaseModel):
    deleted_count: int

@app.get("/", response_class=HTMLResponse)
async def root():
    """Serve the web UI"""
    try:
        with open("web_ui.html", "r") as f:
            return HTMLResponse(content=f.read())
    except FileNotFoundError:
        return HTMLResponse(
            content="<h1>ShrLink Server</h1><p>Web UI not found. Use API endpoints directly.</p>",
            status_code=200
        )

@app.post("/upload", response_model=FileUploadResponse)
async def upload_file(file: UploadFile = File(...)):
    """Upload a file (.shr bundle or any file)"""
    if not file.filename:
        raise HTTPException(status_code=400, detail="No filename provided")
    
    # Generate unique filename to prevent conflicts
    file_extension = Path(file.filename).suffix
    unique_filename = f"{uuid.uuid4()}{file_extension}"
    file_path = UPLOAD_DIR / unique_filename
    
    try:
        # Save the uploaded file
        with open(file_path, "wb") as buffer:
            content = await file.read()
            buffer.write(content)
        
        file_size = len(content)
        download_url = f"http://localhost:8000/files/{unique_filename}"
        
        print(f"📁 Uploaded: {file.filename} -> {unique_filename} ({file_size} bytes)")
        
        return FileUploadResponse(
            status="success",
            filename=unique_filename,
            size=file_size,
            download_url=download_url
        )
    
    except Exception as e:
        if file_path.exists():
            file_path.unlink()
        raise HTTPException(status_code=500, detail=f"Upload failed: {str(e)}")

@app.get("/files/{filename}")
async def download_file(filename: str):
    """Download a file"""
    file_path = UPLOAD_DIR / filename
    
    if not file_path.exists():
        raise HTTPException(status_code=404, detail="File not found")
    
    print(f"📤 Download: {filename}")
    return FileResponse(
        path=file_path,
        filename=filename,
        media_type='application/octet-stream'
    )

@app.get("/stats", response_model=StatsResponse)
async def get_stats():
    """Get server statistics"""
    total_files = 0
    total_bytes = 0
    
    for file_path in UPLOAD_DIR.iterdir():
        if file_path.is_file():
            total_files += 1
            total_bytes += file_path.stat().st_size
    
    return StatsResponse(
        total_files=total_files,
        total_bytes=total_bytes
    )

@app.post("/cleanup", response_model=CleanupResponse)
async def cleanup_files(request: CleanupRequest):
    """Clean up old files"""
    deleted_count = 0
    current_time = time.time()
    
    for file_path in UPLOAD_DIR.iterdir():
        if file_path.is_file():
            file_age = current_time - file_path.stat().st_mtime
            if file_age > request.max_age_seconds:
                file_path.unlink()
                deleted_count += 1
                print(f"🗑️ Deleted old file: {file_path.name}")
    
    return CleanupResponse(deleted_count=deleted_count)

@app.get("/health")
async def health_check():
    """Health check endpoint"""
    return {"status": "healthy", "upload_dir": str(UPLOAD_DIR.absolute())}

# Testing endpoints
@app.post("/test/upload-text")
async def test_upload_text(content: str):
    """Test endpoint: Upload text content as a file"""
    filename = f"test_{uuid.uuid4()}.txt"
    file_path = UPLOAD_DIR / filename
    
    with open(file_path, "w") as f:
        f.write(content)
    
    return {
        "filename": filename,
        "size": len(content.encode()),
        "download_url": f"http://localhost:8000/files/{filename}"
    }

@app.get("/test/shr-demo")
async def test_shr_demo():
    """Demo endpoint showing how ShrLink would work"""
    return {
        "p2p_url": "shr://12D3KooWBhvxmvKvq8Qhd2eNtdqFGRv3YJzjK5xN8mP2Q4rS6tU7/f1c9a8b7e6d5c4b3a2",
        "fallback_url": "http://localhost:8000/files/demo-file.shr",
        "compression_info": {
            "algorithm": "LZ4",
            "block_size": "4 MiB",
            "chunks": 3
        },
        "transfer_methods": [
            "P2P via libp2p (preferred)",
            "HTTP fallback (this server)"
        ]
    }

if __name__ == "__main__":
    print("🚀 Starting ShrLink FastAPI Server")
    print(f"📁 Upload directory: {UPLOAD_DIR.absolute()}")
    print("🌐 Web UI: http://localhost:8000")
    print("📋 API Docs: http://localhost:8000/docs")
    
    uvicorn.run(
        "fastapi_server:app",
        host="0.0.0.0",
        port=8000,
        reload=True,
        log_level="info"
    )
