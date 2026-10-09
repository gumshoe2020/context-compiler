<!-- prompts/distill.md — the distill policy (SETTLE-1b, dev/SPEC-event-driven-settle.md §3, §4). palace distill fills in the person's and the assistant's names (--person/--assistant) and appends the conversation's new text, one line per message as `L<line> [user|assistant] <text>`. The command checks the answer; it does the writing, mining and graph wiring itself. Skip rule: palace distill never sends a conversation with fewer than DISTILL_MIN_TURNS exchanges (default 3); it records the position and files nothing. -->

Memory distillation. Below is the new part of one conversation between {Person} and {Assistant}. Decide what of it is worth remembering long-term, and file it as diary entries, typed claims and graph relations. You have no tools: answer with ONE JSON object and nothing else.

WHAT TO FILE
File only what is worth remembering long-term: decisions made, new facts about people and projects, commitments and dates, preferences {Person} stated, and notable threads that continue (what was tried, what was left open and why). Skip routine chatter, greetings, tool fiddling, status pings and anything already settled elsewhere. If nothing in the conversation is substantive, file nothing: answer {"entries": [], "claims": [], "relations": []}. Filing nothing is a correct answer, not a failure.

ENTRIES
One entry per distinct thread of the conversation, usually one. Each entry: a short title naming the thread; a body of a few plain sentences in the third person, stating what happened and what it means (who decided what, what changed, what is still open); tags: two to five short lowercase words naming the people, the project and the thread (no commas or full stops inside a tag). Never invent anything the conversation does not say.

CLAIMS — TWO KINDS
Each claim is one load-bearing fact, with subject, predicate (active voice), object, the date it is true from (YYYY-MM-DD), a verbatim quote copied exactly from the conversation (no more than 20 words), and the line number L<n> the quote is on.
Classify every claim, by this rule of thumb: if changing one word or digit would make it false, it is a SPECIFIC; otherwise it is a CONCEPT.
- "specific": an exact fact. A date, a name, a number, an amount, a time, a decision, a commitment, or someone's exact words. A specific MUST carry a quote copied character for character from one line of the conversation, and the line it is on. A specific without such a quote is rejected. Never paraphrase a specific: its object states the value as the conversation states it.
- "concept": understanding. Design reasoning, a pattern, a lesson, a theme, a definition. A quote is encouraged. A coined term states its definition and what the term is NOT.
When a specific replaces an earlier value (a new date, a swapped person, a changed amount), state the new value as its own specific with the same subject and predicate; the store keeps the old one as history.

RELATIONS
Graph relations between the entities the claims name: subject, predicate (snake_case, e.g. depends_on, decided, works_on), object, confidence between 0 and 1, and the line number it comes from. Only relations the conversation states directly. None is fine.

ANSWER SHAPE (one JSON object; line numbers are the L numbers below)
{"entries": [{"title": "...", "body": "...", "tags": ["...", "..."]}],
 "claims": [{"kind": "specific", "subject": "...", "predicate": "...", "object": "...", "date": "YYYY-MM-DD", "quote": "...", "line": 12}],
 "relations": [{"subject": "...", "predicate": "...", "object": "...", "confidence": 0.8, "line": 12}]}
