#!/usr/bin/env python3
"""Export client icons from the editable master; --android includes active launcher resources."""
from pathlib import Path
import re
import argparse
import shutil
import subprocess
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / 'macos/AppIcon/RMusicIcon.svg'


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--android',action='store_true',help='Also update the active Tauri Android launcher resources')
    options=parser.parse_args()
    svg = MASTER.read_text()
    # Browser and Windows icon slots supply their own padding; keep the same mark.
    compact = svg.replace('viewBox="0 0 1024 1024"', 'viewBox="100 100 824 824"')
    web = ROOT / 'apps/web/public'
    desktop = ROOT / 'apps/desktop/src-tauri/icons'
    web.mkdir(parents=True, exist_ok=True)
    desktop.mkdir(parents=True, exist_ok=True)
    (web / 'favicon.svg').write_text(compact)
    (desktop / 'icon.svg').write_text(compact)

    def png(source, size, target):
        subprocess.run(['rsvg-convert', '-w', str(size), '-h', str(size), str(source), '-o', str(target)], check=True)

    for name, size in [('favicon-32.png', 32), ('apple-touch-icon.png', 180), ('icon-192.png', 192), ('icon-512.png', 512)]:
        png(web / 'favicon.svg', size, web / name)
    for name, size in [('32x32.png', 32), ('128x128.png', 128), ('128x128@2x.png', 256), ('icon.png', 512)]:
        png(desktop / 'icon.svg', size, desktop / name)
    with Image.open(desktop / 'icon.png') as image:
        image.save(desktop / 'icon.ico', sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    shutil.copyfile(ROOT / 'macos/AppIcon/RMusic.icns', desktop / 'icon.icns')
    index = ROOT / 'apps/web/index.html'
    html = re.sub(r'href="\./favicon\.svg(?:\?[^\"]*)?"', 'href="./favicon.svg?v=rmusic-r1"', index.read_text())
    if 'apple-touch-icon' not in html:
        html = html.replace('<meta name="theme-color"', '<link rel="apple-touch-icon" href="./apple-touch-icon.png?v=rmusic-r1"/><meta name="theme-color"')
    index.write_text(html)
    if options.android:
        res=ROOT/'apps/desktop/src-tauri/gen/android/app/src/main/res'
        mark=re.search(r'<g transform="([^"]+)">(.*?)</g>',svg,re.S)
        assert mark
        foreground='<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024"><g transform="'+mark[1]+'">'+mark[2]+'</g></svg>'
        source=ROOT/'artifacts/development/android-liser-20261031/icon-source';source.mkdir(parents=True,exist_ok=True)
        (source/'foreground.svg').write_text(foreground)
        round_svg=foreground.replace('<g transform=', '<circle cx="512" cy="512" r="512" fill="#171d17"/><g transform=')
        (source/'round.svg').write_text(round_svg)
        for density,scale in [('mdpi',1),('hdpi',1.5),('xhdpi',2),('xxhdpi',3),('xxxhdpi',4)]:
            folder=res/f'mipmap-{density}';folder.mkdir(parents=True,exist_ok=True)
            png(desktop/'icon.svg',int(48*scale),folder/'ic_launcher.png')
            png(source/'round.svg',int(48*scale),folder/'ic_launcher_round.png')
            png(source/'foreground.svg',int(108*scale),folder/'ic_launcher_foreground.png')
        (res/'values/ic_launcher_background.xml').write_text('<resources><color name="ic_launcher_background">#171d17</color></resources>\n')
        path=re.search(r'<path.*?d="([^"]+)"',mark[2]).group(1)
        (res/'drawable/ic_launcher_monochrome.xml').write_text('<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="108dp" android:height="108dp" android:viewportWidth="1024" android:viewportHeight="1024"><group android:scaleX="1.08" android:scaleY="1.08" android:translateX="-66.88" android:translateY="-47.44"><path android:fillColor="#FFFFFF" android:fillType="evenOdd" android:pathData="'+path+'"/></group></vector>\n')
        for version in [26,33]:
            folder=res/f'mipmap-anydpi-v{version}';folder.mkdir(parents=True,exist_ok=True)
            adaptive='<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android"><background android:drawable="@color/ic_launcher_background"/><foreground android:drawable="@mipmap/ic_launcher_foreground"/>'
            if version>=33:adaptive+='<monochrome android:drawable="@drawable/ic_launcher_monochrome"/>'
            adaptive+='</adaptive-icon>\n'
            for name in ['ic_launcher.xml','ic_launcher_round.xml']:(folder/name).write_text(adaptive)
        print('Web / desktop / active Android launcher resources updated.')
    else: print('Web / desktop exports updated. Android res and installed app were not changed.')


if __name__ == '__main__':
    main()
