#!/usr/bin/env python3
"""Real Django feeds for a fixed input -> crates/rango-contrib/tests/golden/feeds.json."""
import datetime, json, os
import django
from django.conf import settings
settings.configure(SECRET_KEY="x", USE_TZ=True)
django.setup()
from django.utils.feedgenerator import Atom1Feed, Rss201rev2Feed, Enclosure
utc = datetime.timezone.utc
out = {}
for name, cls in (("rss", Rss201rev2Feed), ("atom", Atom1Feed)):
    f = cls(title="My <blog>", link="http://example.com/", description="All & more", language="en", author_name="Ann", author_email="ann@example.com", author_link="http://example.com/ann", subtitle="sub", categories=["a", "b"], feed_url="http://example.com/feed/", feed_copyright="(c) me", ttl=30)
    f.add_item("First", "http://example.com/1/#frag", "Body <b>one</b>", author_name="Bob", author_email="bob@example.com", pubdate=datetime.datetime(2024, 1, 5, 12, 0, tzinfo=utc), updateddate=datetime.datetime(2024, 2, 6, 7, 8, 9, tzinfo=utc), comments="http://example.com/1/#c", unique_id="id-1", unique_id_is_permalink=False, categories=["x"], item_copyright="(c) item", enclosures=[Enclosure("http://example.com/a.mp3", "1234", "audio/mpeg")])
    f.add_item("Second", "http://example.com/2/", None, author_name="Cy", pubdate=datetime.datetime(2024, 3, 1, tzinfo=utc))
    out[name] = f.writeString("utf-8")
json.dump(out, open(os.path.join(os.path.dirname(__file__), "..", "crates", "rango-contrib", "tests", "golden", "feeds.json"), "w"), indent=1, ensure_ascii=False)
print("ok")
