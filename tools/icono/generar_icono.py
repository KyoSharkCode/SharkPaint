#!/usr/bin/env python3
"""Genera el icono de SharkPaint (assets/icons/sharkpaint.svg, .png y .ico).

Comparte el sistema del icono de SharkCut (cuadro redondeado azul abisal, colores de la
marca Marea Nocturna) pero se distingue a simple vista: el borde es Lavanda en lugar de
Aleta y el dibujo es un pincel cuyas cerdas tienen forma de aleta, dejando un trazo en
Bioluz.

Uso: python3 tools/icono/generar_icono.py   (necesita Pillow)
"""

import math
import pathlib

from PIL import Image, ImageDraw

RAIZ = pathlib.Path(__file__).resolve().parents[2]
DESTINO = RAIZ / "assets" / "icons"

ABISAL, ALETA, BIOLUZ, LAVANDA, ESPUMA = "#0A1030", "#3A7BD5", "#6FE3F0", "#BDB0E6", "#C9E6F2"
ESCALA, ORIGEN = 7.3, (40, 34)  # rejilla de 24 × 24 dentro del cuadro de 256 px


def bezier(p0, p1, p2, p3, pasos=64):
    """Puntos de una curva cúbica."""
    puntos = []
    for i in range(pasos + 1):
        t = i / pasos
        u = 1 - t
        puntos.append(tuple(u**3 * a + 3 * u * u * t * b + 3 * u * t * t * c + t**3 * d
                            for a, b, c, d in zip(p0, p1, p2, p3)))
    return puntos


def mango(pasos):
    """Mango del pincel: (x, y, radio), algo más fino hacia la virola."""
    return [(20.5 - 7.0 * t, 2.5 + 7.6 * t, 1.25 - 0.25 * t)
            for t in (i / pasos for i in range(pasos + 1))]


def virola(pasos):
    return [(13.5 - 1.8 * t, 10.1 + 1.95 * t, 1.55)
            for t in (i / pasos for i in range(pasos + 1))]


def cerdas(pasos):
    """Las cerdas, con silueta de aleta: polígono cerrado."""
    return bezier((13.3, 12.6), (11.5, 16.5), (7.5, 19.5), (3.2, 20.2), pasos) + \
        bezier((3.2, 20.2), (5.2, 17.5), (7.2, 14.0), (10.4, 10.6), pasos)


def trazo(pasos):
    """El trazo que deja el pincel: fino en los extremos, como con presión."""
    return [(3.4 + 17.6 * t, 20.6 + 0.5 * math.sin(t * math.tau),
             0.1 + 0.75 * math.sin(math.pi * t ** 0.6))
            for t in (i / pasos for i in range(pasos + 1))]


def contorno(puntos):
    """Convierte una línea de grosor variable (x, y, radio) en un polígono cerrado."""
    un_lado, otro_lado = [], []
    for i, (x, y, r) in enumerate(puntos):
        x0, y0, _ = puntos[max(i - 1, 0)]
        x1, y1, _ = puntos[min(i + 1, len(puntos) - 1)]
        largo = math.hypot(x1 - x0, y1 - y0)
        nx, ny = -(y1 - y0) / largo, (x1 - x0) / largo
        un_lado.append((x + nx * r, y + ny * r))
        otro_lado.append((x - nx * r, y - ny * r))
    return un_lado + otro_lado[::-1]


def svg():
    def camino(poligono, color):
        d = " ".join(f"{x:.2f} {y:.2f}" for x, y in poligono)
        return f'    <path d="M{d}Z" fill="{color}"/>'

    def linea(puntos, color):
        (x0, y0, r), (x1, y1, _) = puntos[0], puntos[-1]
        return (f'    <path d="M{x0:.2f} {y0:.2f}L{x1:.2f} {y1:.2f}" stroke="{color}" '
                f'stroke-width="{2 * r:.2f}" stroke-linecap="round"/>')

    partes = [
        camino(contorno(mango(2)), ESPUMA),
        linea(mango(1)[:1] * 2, ESPUMA),  # extremo redondo del mango
        linea(virola(1), ALETA),
        camino(cerdas(40), LAVANDA),
        camino(contorno(trazo(80)), BIOLUZ),
    ]
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
  <rect x="8" y="8" width="240" height="240" rx="56" fill="{ABISAL}" stroke="{LAVANDA}" stroke-width="10"/>
  <g transform="translate({ORIGEN[0]} {ORIGEN[1]}) scale({ESCALA})" fill="none">
{chr(10).join(partes)}
  </g>
</svg>
"""


def dibujar(lado=1024):
    """El icono a `lado` px (se dibuja grande y se reduce: bordes suaves)."""
    k = lado / 256
    imagen = Image.new("RGBA", (lado, lado), (0, 0, 0, 0))
    d = ImageDraw.Draw(imagen)
    d.rounded_rectangle([8 * k, 8 * k, 248 * k, 248 * k], radius=56 * k, fill=ABISAL,
                        outline=LAVANDA, width=round(10 * k))

    def a_px(x, y):
        return ((ORIGEN[0] + ESCALA * x) * k, (ORIGEN[1] + ESCALA * y) * k)

    def discos(puntos, color):
        # Un círculo en cada punto: trazos de extremos redondos y sin dientes.
        for x, y, radio in puntos:
            px, py = a_px(x, y)
            r = radio * ESCALA * k
            d.ellipse([px - r, py - r, px + r, py + r], fill=color)

    discos(mango(600), ESPUMA)
    discos(virola(200), ALETA)
    d.polygon([a_px(x, y) for x, y in cerdas(300)], fill=LAVANDA)
    discos(trazo(800), BIOLUZ)
    return imagen.resize((256, 256), Image.LANCZOS)


def main():
    DESTINO.mkdir(parents=True, exist_ok=True)
    (DESTINO / "sharkpaint.svg").write_text(svg(), encoding="utf-8")
    imagen = dibujar()
    imagen.save(DESTINO / "sharkpaint.png")
    imagen.save(DESTINO / "sharkpaint.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    print(DESTINO / "sharkpaint.ico")


if __name__ == "__main__":
    main()
