# Standard-library IANA timezone conversion for items due; no files or network writes.
import datetime as d
import json
import sys
from zoneinfo import ZoneInfo

epoch = d.datetime(1970, 1, 1, tzinfo=d.timezone.utc)
due_ms, now_ms = map(int, sys.argv[1:3])
zone = ZoneInfo(sys.argv[3])
due = (epoch + d.timedelta(milliseconds=due_ms)).astimezone(zone)
if due.hour >= 22:
    due = d.datetime.combine(due.date() + d.timedelta(days=1), d.time(7), zone)
elif due.hour < 7:
    due = d.datetime.combine(due.date(), d.time(7), zone)
now = (epoch + d.timedelta(milliseconds=now_ms)).astimezone(zone)
print(json.dumps(dict(due=due.astimezone(d.timezone.utc).isoformat(timespec='milliseconds'),
                      cutoff=(now.date() + d.timedelta(days=3)).isoformat())))
