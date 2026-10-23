#!/usr/bin/env python3
"""Writes the small synthetic JPEG test files in tests/resources/imageio/jpg
(dev tooling: no JPEG encoder is available here). Baseline and progressive
(DC successive approximation, AC spectral selection with EOB runs) files,
grayscale and YCbCr with every sampling factor jpgd supports, restart
intervals, 16-bit quantization tables, optimal Huffman tables, extra
markers. Deterministic."""
import math, os, struct, sys

ZAG = [0,1,8,16,9,2,3,10,17,24,32,25,18,11,4,5,12,19,26,33,40,48,41,34,27,20,13,6,7,14,21,28,
       35,42,49,56,57,50,43,36,29,22,15,23,30,37,44,51,58,59,52,45,38,31,39,46,53,60,61,54,47,55,62,63]
LUMA_Q = [16,11,10,16,24,40,51,61,12,12,14,19,26,58,60,55,14,13,16,24,40,57,69,56,14,17,22,29,51,87,80,62,
          18,22,37,56,68,109,103,77,24,35,55,64,81,104,113,92,49,64,78,87,103,121,120,101,72,92,95,98,112,100,103,99]
CHROMA_Q = [17,18,24,47,99,99,99,99,18,21,26,66,99,99,99,99,24,26,56,99,99,99,99,99,47,66,99,99,99,99,99,99] + [99]*32

class Lcg:
    def __init__(self, s): self.s = s & 0xffffffff
    def next(self):
        self.s = (self.s * 1664525 + 1013904223) & 0xffffffff
        return self.s >> 8

def scale_q(q, quality):
    s = 5000 // quality if quality < 50 else 200 - quality * 2
    return [min(max((v * s + 50) // 100, 1), 255) for v in q]

COS = [[math.cos((2 * x + 1) * u * math.pi / 16) for x in range(8)] for u in range(8)]
def fdct(b):
    out = [0.0] * 64
    for v in range(8):
        for u in range(8):
            s = 0.0
            for y in range(8):
                for x in range(8):
                    s += b[y * 8 + x] * COS[u][x] * COS[v][y]
            cu = 1 / math.sqrt(2) if u == 0 else 1
            cv = 1 / math.sqrt(2) if v == 0 else 1
            out[v * 8 + u] = 0.25 * cu * cv * s
    return out

def category(v):
    v = abs(v); n = 0
    while v: n += 1; v >>= 1
    return n

def extra(v, n):
    return v if v >= 0 else v + (1 << n) - 1

class BitWriter:
    def __init__(self): self.out = bytearray(); self.acc = 0; self.n = 0
    def put(self, code, length):
        for i in range(length - 1, -1, -1):
            self.acc = (self.acc << 1) | ((code >> i) & 1); self.n += 1
            if self.n == 8:
                self.out.append(self.acc)
                if self.acc == 0xFF: self.out.append(0)
                self.acc = 0; self.n = 0
    def flush(self):
        if self.n: self.put((1 << (8 - self.n)) - 1, 8 - self.n)

def huff_table(freq):
    """Annex K.2 code lengths (max 16) -> (bits[16], vals)."""
    freq = dict(freq); freq[256] = 1
    syms = list(freq)
    f = {s: freq[s] for s in syms}
    codesize = {s: 0 for s in syms}; others = {s: None for s in syms}
    while True:
        live = [s for s in syms if f[s] > 0]
        if len(live) < 2: break
        live.sort(key=lambda s: (f[s], -s))
        v1 = live[0]; v2 = live[1]
        # K.2: v1 = least freq (largest value on ties), v2 = next
        f[v1] += f[v2]; f[v2] = 0
        codesize[v1] += 1
        while others[v1] is not None: v1 = others[v1]; codesize[v1] += 1
        others[v1] = v2; codesize[v2] += 1
        while others[v2] is not None: v2 = others[v2]; codesize[v2] += 1
    bits = [0] * 33
    for s in syms:
        if codesize[s]: bits[codesize[s]] += 1
    i = 32
    while i > 16:
        while bits[i] > 0:
            j = i - 2
            while bits[j] == 0: j -= 1
            bits[i] -= 2; bits[i - 1] += 1; bits[j + 1] += 2; bits[j] -= 1
        i -= 1
    while bits[i] == 0: i -= 1
    bits[i] -= 1
    vals = sorted([s for s in syms if s != 256 and codesize[s]], key=lambda s: (codesize[s], s))
    return bits[1:17], vals

def codes_from(bits, vals):
    codes = {}; code = 0; k = 0
    for l in range(1, 17):
        for _ in range(bits[l - 1]):
            codes[vals[k]] = (code, l); k += 1; code += 1
        code <<= 1
    return codes

def seg(marker, payload):
    return bytes([0xFF, marker]) + struct.pack('>H', len(payload) + 2) + payload

def make(name, w, h, comps, samp, quality=75, progressive=False, restart=0, q16=False, seed=1, extra_markers=False):
    rng = Lcg(seed)
    # source image
    rgb = []
    for y in range(h):
        for x in range(w):
            n = rng.next() % 24
            rgb.append(((x * 255 // max(w - 1, 1) + n) % 256, (y * 255 // max(h - 1, 1) + 2 * n) % 256,
                        ((x + y) * 7 + n * 3) % 256))
    ncomp = len(comps)
    if ncomp == 1:
        planes = [[0.299 * r + 0.587 * g + 0.114 * b for (r, g, b) in rgb]]
    else:
        planes = [[0.299 * r + 0.587 * g + 0.114 * b for (r, g, b) in rgb],
                  [-0.168736 * r - 0.331264 * g + 0.5 * b + 128 for (r, g, b) in rgb],
                  [0.5 * r - 0.418688 * g - 0.081312 * b + 128 for (r, g, b) in rgb]]
    hmax = max(s[0] for s in samp); vmax = max(s[1] for s in samp)
    mcux = (w + 8 * hmax - 1) // (8 * hmax); mcuy = (h + 8 * vmax - 1) // (8 * vmax)
    qt = [scale_q(LUMA_Q, quality), scale_q(CHROMA_Q, quality)]
    if q16: qt[0] = [min(v * 37, 4000) for v in qt[0]]
    # blocks per component (MCU padded), quantized in natural order
    cblocks = []
    for c in range(ncomp):
        hs, vs = samp[c]
        bw, bh = mcux * hs, mcuy * vs
        cw, ch = (w * hs + hmax - 1) // hmax, (h * vs + vmax - 1) // vmax
        def sample(px, py):
            px = min(px, cw - 1); py = min(py, ch - 1)
            # box filter the source pixels covered by this sample
            sx, sy = hmax // hs, vmax // vs
            tot = 0.0; cnt = 0
            for dy in range(sy):
                for dx in range(sx):
                    X = min(px * sx + dx, w - 1); Y = min(py * sy + dy, h - 1)
                    tot += planes[c][Y * w + X]; cnt += 1
            return tot / cnt
        q = qt[0 if c == 0 else 1]
        blocks = []
        for by in range(bh):
            row = []
            for bx in range(bw):
                b = [sample(bx * 8 + i % 8, by * 8 + i // 8) - 128 for i in range(64)]
                d = fdct(b)
                row.append([int(round(d[i] / q[ZAG.index(i)])) for i in range(64)])
            blocks.append(row)
        cblocks.append((blocks, cw, ch))
    out = bytearray(b'\xff\xd8')
    out += seg(0xE0, b'JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00')
    if extra_markers:
        out += seg(0xFE, b'synthetic test image')
        out += seg(0xE1, b'Exif\x00\x00' + bytes(20))
    for i, t in enumerate(qt[:1 if ncomp == 1 else 2]):
        prec = 1 if (q16 and i == 0) else 0
        payload = bytes([(prec << 4) | i]) + b''.join(struct.pack('>H' if prec else 'B', t[k]) for k in range(64))
        out += seg(0xDB, payload)
    sof = struct.pack('>BHHB', 8, h, w, ncomp)
    for c in range(ncomp):
        sof += bytes([comps[c], (samp[c][0] << 4) | samp[c][1], 0 if c == 0 else 1])
    out += seg(0xC2 if progressive else 0xC0, sof)
    if restart:
        out += seg(0xDD, struct.pack('>H', restart))

    def mcu_blocks(cl, interleaved):
        """(comp, bx, by) per MCU for a scan over components cl."""
        if interleaved:
            for my in range(mcuy):
                for mx in range(mcux):
                    yield [(c, mx * samp[c][0] + i, my * samp[c][1] + j)
                           for c in cl for j in range(samp[c][1]) for i in range(samp[c][0])]
        else:
            c = cl[0]
            cw, ch = cblocks[c][1], cblocks[c][2]
            for by in range((ch + 7) // 8):
                for bx in range((cw + 7) // 8):
                    yield [(c, bx, by)]

    def scan(cl, ss, se, ah, al):
        interleaved = len(cl) > 1
        # pass 1: symbols, pass 2: encode
        for pas in (0, 1):
            dcf = [{} for _ in range(2)]; acf = [{} for _ in range(2)]
            bw = BitWriter(); pred = [0] * ncomp; eobrun = [0]; n_mcu = 0
            tab = lambda c: 0 if c == 0 else 1
            def emit_dc(c, v):
                n = category(v)
                if pas == 0: dcf[tab(c)][n] = dcf[tab(c)].get(n, 0) + 1
                else:
                    code, l = dcc[tab(c)][n]; bw.put(code, l)
                    if n: bw.put(extra(v, n), n)
            def emit_ac(c, sym, val, n):
                if pas == 0: acf[tab(c)][sym] = acf[tab(c)].get(sym, 0) + 1
                else:
                    code, l = acc[tab(c)][sym]; bw.put(code, l)
                    if n: bw.put(val, n)
            def flush_eob(c):
                if eobrun[0]:
                    r = category(eobrun[0]) - 1
                    emit_ac(c, r << 4, eobrun[0] - (1 << r), r)
                    eobrun[0] = 0
            for blocks in mcu_blocks(cl, interleaved):
                if restart and not progressive and n_mcu and n_mcu % restart == 0 and pas == 1:
                    bw.flush(); bw.out += bytes([0xFF, 0xD0 + ((n_mcu // restart - 1) & 7)])
                if restart and not progressive and n_mcu and n_mcu % restart == 0:
                    pred = [0] * ncomp
                n_mcu += 1
                for (c, bx, by) in blocks:
                    co = cblocks[c][0][by][bx]
                    if ss == 0:
                        if ah == 0:
                            v = co[0] >> al
                            emit_dc(c, v - pred[c]); pred[c] = v
                        elif pas == 1:
                            bw.put((co[0] >> al) & 1, 1)
                        if se == 0: continue
                    zz = [co[ZAG[k]] for k in range(64)]
                    start = max(ss, 1)
                    if progressive:
                        vals = [(abs(zz[k]) >> al) * (1 if zz[k] >= 0 else -1) for k in range(64)]
                        last = max([k for k in range(start, se + 1) if vals[k]] or [0])
                        if last == 0:
                            eobrun[0] += 1
                            if eobrun[0] == 0x7FFF: flush_eob(c)
                            continue
                        flush_eob(c)
                        r = 0
                        for k in range(start, last + 1):
                            if vals[k] == 0: r += 1; continue
                            while r > 15: emit_ac(c, 0xF0, 0, 0); r -= 16
                            n = category(vals[k]); emit_ac(c, (r << 4) | n, extra(vals[k], n), n); r = 0
                        if last < se: eobrun[0] += 1
                    else:
                        r = 0
                        for k in range(1, 64):
                            if zz[k] == 0: r += 1; continue
                            while r > 15: emit_ac(c, 0xF0, 0, 0); r -= 16
                            n = category(zz[k]); emit_ac(c, (r << 4) | n, extra(zz[k], n), n); r = 0
                        if r: emit_ac(c, 0x00, 0, 0)
            if progressive and se > 0: flush_eob(cl[0])
            if pas == 0:
                tables = bytearray(); dcc = [None, None]; acc = [None, None]
                for t in range(2):
                    if dcf[t]:
                        bits, vals = huff_table(dcf[t]); dcc[t] = codes_from(bits, vals)
                        tables += bytes([t]) + bytes(bits) + bytes(vals)
                    if acf[t]:
                        bits, vals = huff_table(acf[t]); acc[t] = codes_from(bits, vals)
                        tables += bytes([0x10 | t]) + bytes(bits) + bytes(vals)
                hdr = seg(0xC4, bytes(tables)) if tables else b''
            else:
                bw.flush()
                sos = bytes([len(cl)])
                for c in cl:
                    sos += bytes([comps[c], (tab(c) << 4) | tab(c)])
                sos += bytes([ss, se, (ah << 4) | al])
                return hdr + seg(0xDA, sos) + bytes(bw.out)

    allc = list(range(ncomp))
    if not progressive:
        out += scan(allc, 0, 63, 0, 0)
    else:
        out += scan(allc, 0, 0, 0, 1)
        for c in allc:
            out += scan([c], 1, 5, 0, 0)
        out += scan(allc, 0, 0, 1, 0)
        for c in allc:
            out += scan([c], 6, 63, 0, 0)
    out += b'\xff\xd9'
    return bytes(out)

def main():
    d = sys.argv[1] if len(sys.argv) > 1 else 'tests/resources/imageio/jpg'
    os.makedirs(d, exist_ok=True)
    YCC = [1, 2, 3]
    cases = {
        'gray_17x9': (17, 9, [1], [(1, 1)], {}),
        'gray_1x1': (1, 1, [1], [(1, 1)], {}),
        'h1v1_33x31': (33, 31, YCC, [(1, 1), (1, 1), (1, 1)], {}),
        'h2v1_33x31': (33, 31, YCC, [(2, 1), (1, 1), (1, 1)], {}),
        'h1v2_33x31': (33, 31, YCC, [(1, 2), (1, 1), (1, 1)], {}),
        'h2v2_33x31': (33, 31, YCC, [(2, 2), (1, 1), (1, 1)], {}),
        'h2v2_1x1': (1, 1, YCC, [(2, 2), (1, 1), (1, 1)], {}),
        'h2v2_2x2': (2, 2, YCC, [(2, 2), (1, 1), (1, 1)], {}),
        'h2v2_47x5': (47, 5, YCC, [(2, 2), (1, 1), (1, 1)], {}),
        'h1v2_9x40': (9, 40, YCC, [(1, 2), (1, 1), (1, 1)], {}),
        'h2v1_40x3': (40, 3, YCC, [(2, 1), (1, 1), (1, 1)], {}),
        'h2v2_q95_restart': (40, 24, YCC, [(2, 2), (1, 1), (1, 1)], {'quality': 95, 'restart': 2}),
        'h1v1_restart1': (24, 16, YCC, [(1, 1), (1, 1), (1, 1)], {'restart': 1}),
        'gray_q16': (16, 16, [1], [(1, 1)], {'q16': True}),
        'h2v2_q10_markers': (32, 32, YCC, [(2, 2), (1, 1), (1, 1)], {'quality': 10, 'extra_markers': True}),
        'prog_gray_19x13': (19, 13, [1], [(1, 1)], {'progressive': True}),
        'prog_h1v1_21x18': (21, 18, YCC, [(1, 1), (1, 1), (1, 1)], {'progressive': True}),
        'prog_h2v1_30x20': (30, 20, YCC, [(2, 1), (1, 1), (1, 1)], {'progressive': True}),
        'prog_h1v2_20x30': (20, 30, YCC, [(1, 2), (1, 1), (1, 1)], {'progressive': True}),
        'prog_h2v2_35x33': (35, 33, YCC, [(2, 2), (1, 1), (1, 1)], {'progressive': True, 'quality': 90}),
        'prog_h2v2_1x1': (1, 1, YCC, [(2, 2), (1, 1), (1, 1)], {'progressive': True}),
    }
    for i, (name, (w, h, comps, samp, kw)) in enumerate(sorted(cases.items())):
        data = make(name, w, h, comps, samp, seed=i + 1, **kw)
        open(os.path.join(d, name + '.jpg'), 'wb').write(data)
    print(len(cases), 'files')

main()
