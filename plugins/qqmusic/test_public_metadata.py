import unittest
from public_metadata import describe, parse, singer_person, singer_image, artwork_identity


class PublicMetadata(unittest.TestCase):
    def test_missing_photo_is_explicit_and_default_silhouette_is_not_an_artist_photo(self):
        person = {'basic_info': {'singer_mid': 'Abc123', 'has_photo': 0}, 'pic': {'pic': 'http://y.gtimg.cn/music/photo_new/T001R300x300M000Default_0.jpg'}}
        raw = {'code': 0, 'req': {'code': 0, 'data': {'singer_list': [person]}}}
        result = parse('artist', 'Abc123', raw)
        self.assertIsNone(result['image_url'])
        self.assertEqual(result['image_status'], 'unavailable')
        self.assertNotIn('image_status', singer_image({'basic_info': {}}, 'Abc123'))
        person['basic_info']['has_photo'] = 1
        person['pic']['pic'] = 'http://y.gtimg.cn/music/photo_new/T001R300x300M000Actual_0.jpg'
        self.assertEqual(singer_image(person, 'Abc123')['image_url'], 'https://y.gtimg.cn/music/photo_new/T001R300x300M000Actual_0.jpg')
        self.assertEqual(artwork_identity('https://y.gtimg.cn/music/photo_new/T001R300x300M000Abc123.jpg'), 'Abc123')
        for url in ('https://evil.test/music/photo_new/T001R300x300M000Abc123.jpg', 'https://y.gtimg.cn/music/photo_new/T002R300x300M000Abc123.jpg', 'https://user@y.gtimg.cn/music/photo_new/T001R300x300M000Abc123.jpg'):
            self.assertIsNone(artwork_identity(url))
        with self.assertRaises(ValueError):
            singer_person({'singer_list': [{'basic_info': {'mid': 'Wrong123'}}]}, 'Abc123')

    def test_anonymous_request_contains_no_account_and_is_identity_bound(self):
        for kind in ('artist', 'album'):
            request = describe(kind, 'Abc123')
            self.assertEqual(request['json']['comm']['uin'], 0)
            self.assertNotIn('cookies', request)
            self.assertNotIn('headers', request)
        for kind, mid in [('track', 'Abc123'), ('artist', '../x'), ('album', '')]:
            with self.assertRaises(ValueError): describe(kind, mid)

    def test_real_wire_schema_and_identity_mismatch(self):
        album = {'code': 0, 'req': {'code': 0, 'data': {'basicInfo': {'albumMid': 'Abc123', 'desc': '<p>Album</p>', 'publishDate': '2026-01-01'}, 'company': {'name': 'Label'}}}}
        result = parse('album', 'Abc123', album)
        self.assertEqual(result['description'], 'Album')
        self.assertEqual(result['release_date'], '2026-01-01')
        self.assertEqual(result['label'], 'Label')
        with self.assertRaises(ValueError): parse('album', 'Other', album)
        artist = {'code': 0, 'req': {'code': 0, 'data': {'singer_list': [{'singer_mid': 'Abc123', 'ex_info': {'desc': '<p>Singer</p>', 'genre': '19'}}]}}}
        self.assertEqual(parse('artist', 'Abc123', artist)['description'], 'Singer')
        self.assertNotIn('genre', parse('artist', 'Abc123', artist))
        with self.assertRaises(ValueError): parse('artist', 'Other', artist)
        with self.assertRaises(ValueError): parse('artist', 'Abc123', {'code': 500})
