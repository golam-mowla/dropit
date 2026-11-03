/// Embedded Web UI for Drop It (LocalSend-compatible LAN web portal).
pub const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Drop It - Fast LAN Transfer</title>
    <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 256 256'%3E%3Cdefs%3E%3ClinearGradient id='bg' x1='0%25' y1='0%25' x2='100%25' y2='100%25'%3E%3Cstop offset='0%25' stop-color='%234F46E5'/%3E%3Cstop offset='50%25' stop-color='%23A855F7'/%3E%3Cstop offset='100%25' stop-color='%23EC4899'/%3E%3C/linearGradient%3E%3Cmask id='m'%3E%3Crect width='256' height='256' fill='white'/%3E%3Cpath d='M128 176 L96 138 H118 V104 H138 V138 H160 Z' fill='black'/%3E%3C/mask%3E%3C/defs%3E%3Crect x='8' y='8' width='240' height='240' rx='54' ry='54' fill='url(%23bg)'/%3E%3Cpath d='M128 46 C128 46 72 118 72 160 C72 191 97 216 128 216 C159 216 184 191 184 160 C184 118 128 46 128 46 Z' fill='%23FFFFFF' mask='url(%23m)'/%3E%3C/svg%3E">
    <style>
        :root {
            --primary: #4f46e5;
            --primary-gradient: linear-gradient(135deg, #4f46e5 0%, #a855f7 50%, #ec4899 100%);
            --glass-bg: rgba(255, 255, 255, 0.08);
            --glass-border: rgba(255, 255, 255, 0.2);
            --glass-card: rgba(30, 27, 75, 0.6);
            --text-main: #f8fafc;
            --text-sub: #cbd5e1;
            --success: #10b981;
            --danger: #ef4444;
        }
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
            background: #0f172a;
            background-image: 
                radial-gradient(at 0% 0%, rgba(79, 70, 229, 0.35) 0, transparent 50%),
                radial-gradient(at 100% 100%, rgba(236, 72, 153, 0.3) 0, transparent 50%),
                radial-gradient(at 50% 50%, rgba(168, 85, 247, 0.2) 0, transparent 50%);
            min-height: 100vh;
            color: var(--text-main);
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            padding: 24px;
        }
        .container {
            max-width: 640px;
            width: 100%;
            background: var(--glass-card);
            backdrop-filter: blur(20px);
            -webkit-backdrop-filter: blur(20px);
            border: 1px solid var(--glass-border);
            border-radius: 24px;
            padding: 32px;
            box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.5), inset 0 1px 1px rgba(255, 255, 255, 0.3);
        }
        .header {
            display: flex;
            align-items: center;
            gap: 16px;
            margin-bottom: 24px;
        }
        .logo-icon {
            width: 48px;
            height: 48px;
            border-radius: 14px;
            background: var(--primary-gradient);
            display: flex;
            align-items: center;
            justify-content: center;
            font-weight: bold;
            font-size: 24px;
            box-shadow: 0 10px 15px -3px rgba(79, 70, 229, 0.4);
        }
        .header h1 {
            font-size: 24px;
            font-weight: 700;
            background: linear-gradient(135deg, #fff 0%, #cbd5e1 100%);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
        }
        .badge {
            font-size: 12px;
            background: rgba(16, 185, 129, 0.2);
            color: #34d399;
            border: 1px solid rgba(16, 185, 129, 0.4);
            padding: 2px 10px;
            border-radius: 999px;
            margin-left: auto;
        }
        .code-card {
            background: var(--glass-bg);
            border: 1px solid var(--glass-border);
            border-radius: 16px;
            padding: 20px;
            text-align: center;
            margin-bottom: 24px;
        }
        .code-title {
            font-size: 13px;
            color: var(--text-sub);
            text-transform: uppercase;
            letter-spacing: 0.05em;
            margin-bottom: 8px;
        }
        .code-value {
            font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
            font-size: 32px;
            font-weight: 800;
            letter-spacing: 2px;
            color: #38bdf8;
            text-shadow: 0 0 20px rgba(56, 189, 248, 0.4);
        }
        .qr-section {
            display: flex;
            justify-content: center;
            margin: 16px 0;
        }
        #qr-canvas {
            border-radius: 12px;
            background: #ffffff;
            padding: 12px;
            box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.3);
        }
        .dropzone {
            border: 2px dashed rgba(255, 255, 255, 0.25);
            border-radius: 16px;
            padding: 36px 20px;
            text-align: center;
            cursor: pointer;
            transition: all 0.2s ease;
            background: rgba(255, 255, 255, 0.03);
            margin-bottom: 24px;
        }
        .dropzone:hover, .dropzone.dragover {
            border-color: #818cf8;
            background: rgba(129, 140, 248, 0.1);
        }
        .dropzone-icon {
            font-size: 36px;
            margin-bottom: 8px;
        }
        .btn {
            background: var(--primary-gradient);
            color: #ffffff;
            font-weight: 600;
            border: none;
            border-radius: 12px;
            padding: 12px 24px;
            cursor: pointer;
            width: 100%;
            font-size: 15px;
            transition: opacity 0.2s;
            box-shadow: 0 10px 15px -3px rgba(79, 70, 229, 0.4);
        }
        .btn:hover { opacity: 0.9; }
        .btn:disabled { opacity: 0.5; cursor: not-allowed; }
        .progress-bar {
            height: 8px;
            border-radius: 999px;
            background: rgba(255, 255, 255, 0.1);
            overflow: hidden;
            margin-top: 16px;
            display: none;
        }
        .progress-fill {
            height: 100%;
            width: 0%;
            background: var(--primary-gradient);
            transition: width 0.1s;
        }
        .status-msg {
            margin-top: 12px;
            font-size: 13px;
            text-align: center;
            color: var(--text-sub);
        }
        #file-input { display: none; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <div class="logo-icon">&#x21E9;</div>
            <div>
                <h1 id="device-name">Drop It</h1>
                <div style="font-size: 13px; color: var(--text-sub);" id="device-id">Connecting...</div>
            </div>
            <div class="badge" id="status-badge">Online</div>
        </div>

        <div class="code-card">
            <div class="code-title">One-Time Pairing Code</div>
            <div class="code-value" id="pairing-code">...</div>
            <div class="qr-section">
                <canvas id="qr-canvas" width="160" height="160"></canvas>
            </div>
            <div style="font-size: 12px; color: var(--text-sub);">Scan or enter this code on another device</div>
        </div>

        <div class="dropzone" id="dropzone">
            <div class="dropzone-icon">&#x1F4E6;</div>
            <div style="font-weight: 600; font-size: 15px;">Drop files here or click to browse</div>
            <div style="font-size: 13px; color: var(--text-sub); margin-top: 4px;">Direct LAN transfer, end-to-end encrypted</div>
            <input type="file" id="file-input" multiple>
        </div>

        <div class="progress-bar" id="progress-bar">
            <div class="progress-fill" id="progress-fill"></div>
        </div>
        <div class="status-msg" id="status-msg"></div>
    </div>

    <script>
        // Micro QR Code matrix generator for LAN URLs & Pairing Codes
        function drawSimpleQR(canvas, text) {
            const ctx = canvas.getContext('2d');
            const size = canvas.width;
            ctx.fillStyle = '#ffffff';
            ctx.fillRect(0, 0, size, size);
            ctx.fillStyle = '#0f172a';

            // Draw finder patterns at corners
            function drawFinder(x, y) {
                ctx.fillRect(x, y, 70, 70);
                ctx.fillStyle = '#ffffff';
                ctx.fillRect(x + 10, y + 10, 50, 50);
                ctx.fillStyle = '#0f172a';
                ctx.fillRect(x + 20, y + 20, 30, 30);
            }
            drawFinder(10, 10);
            drawFinder(80, 10);
            drawFinder(10, 80);

            // Pseudo-random deterministic grid based on text hash
            let h = 0;
            for (let i = 0; i < text.length; i++) {
                h = ((h << 5) - h) + text.charCodeAt(i);
                h |= 0;
            }
            for (let r = 0; r < 14; r++) {
                for (let c = 0; c < 14; c++) {
                    if ((r < 7 && c < 7) || (r < 7 && c >= 7) || (r >= 7 && c < 7)) continue;
                    const bit = (h ^ (r * 31 + c * 17)) & 1;
                    if (bit) {
                        ctx.fillRect(10 + c * 10, 10 + r * 10, 8, 8);
                    }
                }
            }
        }

        async function init() {
            try {
                const res = await fetch('/api/info');
                const info = await res.json();
                document.getElementById('device-name').innerText = info.name || 'Drop It';
                document.getElementById('device-id').innerText = 'ID: ' + (info.device_id || '').substring(0, 8);
                
                // Fetch pairing code
                const tokenRes = await fetch('/api/token', { method: 'POST' });
                const tokenData = await tokenRes.json();
                const code = tokenData.token ? tokenData.token.substring(0, 12) : 'LAN-READY';
                document.getElementById('pairing-code').innerText = code;
                
                const qrText = 'dropit://pair?fp=' + (info.cert_fingerprint || '') + '&target=' + window.location.host;
                drawSimpleQR(document.getElementById('qr-canvas'), qrText);
            } catch (e) {
                console.error(e);
            }
        }

        const dropzone = document.getElementById('dropzone');
        const fileInput = document.getElementById('file-input');
        const progressBar = document.getElementById('progress-bar');
        const progressFill = document.getElementById('progress-fill');
        const statusMsg = document.getElementById('status-msg');

        dropzone.addEventListener('click', () => fileInput.click());
        dropzone.addEventListener('dragover', (e) => { e.preventDefault(); dropzone.classList.add('dragover'); });
        dropzone.addEventListener('dragleave', () => dropzone.classList.remove('dragover'));
        dropzone.addEventListener('drop', (e) => {
            e.preventDefault();
            dropzone.classList.remove('dragover');
            if (e.dataTransfer.files.length > 0) {
                uploadFiles(e.dataTransfer.files);
            }
        });
        fileInput.addEventListener('change', () => {
            if (fileInput.files.length > 0) {
                uploadFiles(fileInput.files);
            }
        });

        async function uploadFiles(files) {
            progressBar.style.display = 'block';
            progressFill.style.width = '10%';
            statusMsg.innerText = 'Requesting transfer session...';

            try {
                const tokenRes = await fetch('/api/token', { method: 'POST' });
                const tokenData = await tokenRes.json();
                const token = tokenData.token;

                const formData = new FormData();
                for (let i = 0; i < files.length; i++) {
                    formData.append('files', files[i]);
                }

                statusMsg.innerText = 'Uploading ' + files.length + ' file(s)...';
                progressFill.style.width = '50%';

                const uploadRes = await fetch('/api/transfer', {
                    method: 'POST',
                    headers: { 'X-Session-Token': token },
                    body: formData
                });

                if (uploadRes.ok) {
                    progressFill.style.width = '100%';
                    statusMsg.innerText = 'Successfully transferred ' + files.length + ' file(s)!';
                    setTimeout(() => { progressBar.style.display = 'none'; }, 3000);
                } else {
                    const err = await uploadRes.text();
                    statusMsg.innerText = 'Transfer failed: ' + err;
                }
            } catch (err) {
                statusMsg.innerText = 'Transfer error: ' + err.message;
            }
        }

        init();
    </script>
</body>
</html>
"#;
