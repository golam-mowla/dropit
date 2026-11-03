Add-Type -AssemblyName System.Drawing

$csSource = @"
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Collections.Generic;

public class IconGenerator {
    public static Bitmap GenerateDropItBitmap(int size) {
        Bitmap bmp = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (Graphics g = Graphics.FromImage(bmp)) {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            g.Clear(Color.Transparent);

            // 1. Background gradient squircle
            float margin = size * 0.04f;
            float w = size - (margin * 2);
            float radius = size * 0.22f;
            RectangleF rect = new RectangleF(margin, margin, w, w);

            using (GraphicsPath path = new GraphicsPath()) {
                float d = radius * 2;
                path.AddArc(rect.X, rect.Y, d, d, 180, 90);
                path.AddArc(rect.Right - d, rect.Y, d, d, 270, 90);
                path.AddArc(rect.Right - d, rect.Bottom - d, d, d, 0, 90);
                path.AddArc(rect.X, rect.Bottom - d, d, d, 90, 90);
                path.CloseFigure();

                using (LinearGradientBrush gradBrush = new LinearGradientBrush(
                    new PointF(rect.X, rect.Y),
                    new PointF(rect.Right, rect.Bottom),
                    Color.FromArgb(255, 79, 70, 229),
                    Color.FromArgb(255, 236, 72, 153))) {

                    ColorBlend blend = new ColorBlend(3);
                    blend.Colors = new Color[] {
                        Color.FromArgb(255, 79, 70, 229),
                        Color.FromArgb(255, 168, 85, 247),
                        Color.FromArgb(255, 236, 72, 153)
                    };
                    blend.Positions = new float[] { 0.0f, 0.5f, 1.0f };
                    gradBrush.InterpolationColors = blend;
                    g.FillPath(gradBrush, path);
                }

                // 2. Inner border 1px white 35% opacity
                using (Pen borderPen = new Pen(Color.FromArgb(90, 255, 255, 255), Math.Max(1.0f, size * 0.015f))) {
                    g.DrawPath(borderPen, path);
                }

                // Soft top highlight
                using (LinearGradientBrush topHighlight = new LinearGradientBrush(
                    new PointF(0, margin),
                    new PointF(0, margin + size * 0.35f),
                    Color.FromArgb(100, 255, 255, 255),
                    Color.FromArgb(0, 255, 255, 255))) {
                    using (GraphicsPath topPath = new GraphicsPath()) {
                        topPath.AddArc(rect.X, rect.Y, d, d, 180, 90);
                        topPath.AddArc(rect.Right - d, rect.Y, d, d, 270, 90);
                        topPath.AddLine(rect.Right, margin + size * 0.35f, rect.X, margin + size * 0.35f);
                        topPath.CloseFigure();
                        g.FillPath(topHighlight, topPath);
                    }
                }
            }

            // 3. Water drop shape
            float cx = size * 0.5f;
            float topY = size * 0.20f;
            float centerCircleY = size * 0.60f;
            float circleR = size * 0.22f;

            using (GraphicsPath dropPath = new GraphicsPath()) {
                dropPath.AddLine(cx, topY, cx - circleR * 0.95f, centerCircleY - size * 0.05f);
                dropPath.AddArc(cx - circleR, centerCircleY - circleR, circleR * 2, circleR * 2, 140, 260);
                dropPath.CloseFigure();

                using (SolidBrush whiteBrush = new SolidBrush(Color.White)) {
                    g.FillPath(whiteBrush, dropPath);
                }
            }

            // 4. Downward arrow, matching the app-icon preview.
            float aw = size * 0.18f;
            float stemW = size * 0.08f;
            float stemH = size * 0.11f;
            float headH = size * 0.12f;
            float startY = centerCircleY - size * 0.14f;

            using (GraphicsPath arrowPath = new GraphicsPath()) {
                arrowPath.AddRectangle(new RectangleF(cx - stemW / 2, startY, stemW, stemH));
                arrowPath.AddPolygon(new PointF[] {
                    new PointF(cx - aw / 2, startY + stemH),
                    new PointF(cx + aw / 2, startY + stemH),
                    new PointF(cx, startY + stemH + headH)
                });

                using (Pen arrowPen = new Pen(Color.FromArgb(255, 124, 58, 237), Math.Max(2.0f, size * 0.055f))) {
                    arrowPen.StartCap = LineCap.Round;
                    arrowPen.EndCap = LineCap.Round;
                    arrowPen.LineJoin = LineJoin.Round;
                    g.DrawLine(arrowPen, new PointF(cx, startY), new PointF(cx, startY + stemH + headH * 0.35f));
                    g.DrawLine(arrowPen, new PointF(cx - aw / 2, startY + stemH), new PointF(cx, startY + stemH + headH));
                    g.DrawLine(arrowPen, new PointF(cx, startY + stemH + headH), new PointF(cx + aw / 2, startY + stemH));
                }
            }
        }
        return bmp;
    }

    public static Bitmap GenerateTrayBitmap(int size, string dot) {
        Bitmap bmp = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (Graphics g = Graphics.FromImage(bmp)) {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.Clear(Color.Transparent);

            float cx = size * 0.5f;
            float topY = size * 0.12f;
            float centerCircleY = size * 0.60f;
            float circleR = size * 0.30f;

            using (GraphicsPath dropPath = new GraphicsPath()) {
                dropPath.AddLine(cx, topY, cx - circleR * 0.95f, centerCircleY - size * 0.05f);
                dropPath.AddArc(cx - circleR, centerCircleY - circleR, circleR * 2, circleR * 2, 140, 260);
                dropPath.CloseFigure();

                using (SolidBrush whiteBrush = new SolidBrush(Color.White)) {
                    g.FillPath(whiteBrush, dropPath);
                }
            }

            // Arrow cutout
            float aw = size * 0.28f;
            float stemW = size * 0.12f;
            float stemH = size * 0.14f;
            float headH = size * 0.16f;
            float startY = centerCircleY - size * 0.18f;

            using (GraphicsPath arrowPath = new GraphicsPath()) {
                arrowPath.AddRectangle(new RectangleF(cx - stemW / 2, startY, stemW, stemH));
                arrowPath.AddPolygon(new PointF[] {
                    new PointF(cx - aw / 2, startY + stemH),
                    new PointF(cx + aw / 2, startY + stemH),
                    new PointF(cx, startY + stemH + headH)
                });

                using (SolidBrush blackBrush = new SolidBrush(Color.FromArgb(255, 15, 23, 42))) {
                    g.FillPath(blackBrush, arrowPath);
                }
            }

            if (dot == "connected") {
                using (SolidBrush dotBrush = new SolidBrush(Color.FromArgb(255, 34, 197, 94))) {
                    g.FillEllipse(dotBrush, size * 0.65f, size * 0.65f, size * 0.32f, size * 0.32f);
                }
            } else if (dot == "unread") {
                using (SolidBrush dotBrush = new SolidBrush(Color.FromArgb(255, 79, 70, 229))) {
                    g.FillEllipse(dotBrush, size * 0.65f, size * 0.65f, size * 0.32f, size * 0.32f);
                }
            }
        }
        return bmp;
    }

    public static void GenerateAll(string baseDir) {
        int[] sizes = new int[] { 16, 24, 32, 48, 64, 128, 256 };
        List<byte[]> pngList = new List<byte[]>();

        foreach (int s in sizes) {
            using (Bitmap b = GenerateDropItBitmap(s)) {
                using (MemoryStream ms = new MemoryStream()) {
                    b.Save(ms, ImageFormat.Png);
                    byte[] bytes = ms.ToArray();
                    pngList.Add(bytes);
                    if (s == 128) File.WriteAllBytes(Path.Combine(baseDir, "logo.png"), bytes);
                    if (s == 256) File.WriteAllBytes(Path.Combine(baseDir, "icon-256.png"), bytes);
                    if (s == 32) File.WriteAllBytes(Path.Combine(baseDir, "icon-32.png"), bytes);
                    if (s == 16) File.WriteAllBytes(Path.Combine(baseDir, "icon-16.png"), bytes);
                }
            }
        }

        using (Bitmap t16 = GenerateTrayBitmap(16, ""))
            t16.Save(Path.Combine(baseDir, "icon-tray-16.png"), ImageFormat.Png);
        using (Bitmap t32 = GenerateTrayBitmap(32, ""))
            t32.Save(Path.Combine(baseDir, "icon-tray-32.png"), ImageFormat.Png);
        using (Bitmap tConn = GenerateTrayBitmap(32, "connected"))
            tConn.Save(Path.Combine(baseDir, "icon-tray-connected.png"), ImageFormat.Png);
        using (Bitmap tUnread = GenerateTrayBitmap(32, "unread"))
            tUnread.Save(Path.Combine(baseDir, "icon-tray-unread.png"), ImageFormat.Png);

        // Export raw RGBA for 32x32 window icon
        using (Bitmap bmp32 = GenerateDropItBitmap(32)) {
            byte[] rgba32 = new byte[32 * 32 * 4];
            int p = 0;
            for (int y = 0; y < 32; y++) {
                for (int x = 0; x < 32; x++) {
                    Color c = bmp32.GetPixel(x, y);
                    rgba32[p++] = c.R;
                    rgba32[p++] = c.G;
                    rgba32[p++] = c.B;
                    rgba32[p++] = c.A;
                }
            }
            File.WriteAllBytes(Path.Combine(baseDir, "icon-32.rgba"), rgba32);
        }

        using (MemoryStream icoStream = new MemoryStream()) {
            using (BinaryWriter w = new BinaryWriter(icoStream)) {
                w.Write((ushort)0);
                w.Write((ushort)1);
                w.Write((ushort)sizes.Length);

                int headerSize = 6 + (16 * sizes.Length);
                int currentOffset = headerSize;

                for (int i = 0; i < sizes.Length; i++) {
                    int s = sizes[i];
                    byte[] bytes = pngList[i];
                    w.Write((byte)(s == 256 ? 0 : s));
                    w.Write((byte)(s == 256 ? 0 : s));
                    w.Write((byte)0);
                    w.Write((byte)0);
                    w.Write((ushort)1);
                    w.Write((ushort)32);
                    w.Write((uint)bytes.Length);
                    w.Write((uint)currentOffset);
                    currentOffset += bytes.Length;
                }

                for (int i = 0; i < sizes.Length; i++) {
                    w.Write(pngList[i]);
                }

                File.WriteAllBytes(Path.Combine(baseDir, "icon.ico"), icoStream.ToArray());
            }
        }
    }
}
"@

Add-Type -TypeDefinition $csSource -ReferencedAssemblies System.Drawing
[IconGenerator]::GenerateAll("assets")
Write-Output "Successfully generated assets/icon.ico and PNG assets!"
