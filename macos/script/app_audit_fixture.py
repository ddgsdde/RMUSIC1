"""Small synthetic music library for native UI/visual acceptance. No user media or credentials."""
import pathlib, subprocess, time
from verify_features import png


def seed_music(work, api, token):
    music = work / 'music'
    music.mkdir()
    samples = [('星河慢行', 'AFTERGLOW', 48000, 24), ('夜航', 'MIDNIGHT', 44100, 16),
               ('山海之间', 'DISTANCE', 96000, 24), ('无声片刻', 'QUIET', 48000, 24)]
    for index, (title, album, rate, bits) in enumerate(samples):
        folder = music / album
        folder.mkdir()
        png(folder / 'cover.png', index)
        audio = folder / (title + '.flac')
        subprocess.run(['/opt/homebrew/bin/ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
            f'sine=frequency={440 + index * 110}:sample_rate={rate}:duration=600', '-ac', '2',
            '-af', 'volume=0.005', '-c:a', 'flac', '-sample_fmt', 's16' if bits == 16 else 's32',
            '-metadata', 'title=' + title, '-metadata', 'artist=R-Music Studio · 合成验收',
            '-metadata', 'album=' + album, '-metadata', 'album_artist=R-Music Studio', str(audio)], check=True)
        text = ['星河在此刻流淌', '每个瞬间都有回响', '音乐划过安静的夜', 'A moment held in sound']
        lines = [f'[{i*6//60:02}:{i*6%60:02}.00]<{i*6//60:02}:{i*6%60:02}.00>{text[i%4][:3]}<{i*6//60:02}:{i*6%60+2:02}.00>{text[i%4][3:]}' for i in range(90)]
        audio.with_suffix('.lrc').write_text('\n'.join(lines))
    request = lambda path, body=None: api(path, token, body)
    request('/library/roots', {'path': str(music)})
    job = request('/library/scan', {})['id']
    for _ in range(200):
        if request('/jobs?id=' + job + '&limit=1')['items'][0]['status'] == 'completed': break
        time.sleep(.1)
    else: raise RuntimeError('Synthetic library scan timed out')
    tracks = [t for t in request('/library/tracks?limit=50')['items'] if t['source'] == 'local']
    assert len(tracks) == 4 and all(t['cover_url'] for t in tracks)
    request('/playlists', {'name': '舞台验收 · 四首原文件', 'track_ids': [t['id'] for t in tracks]})
    return {'tracks': len(tracks), 'physical_output_opened': False, 'audio_cache_enabled': False}
