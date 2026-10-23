// Shared canonical source, copied into each packaged Go bridge at build time.
// No engine or platform types cross this boundary.
package main

import (
	"encoding/json"
	"errors"
	"math"
	"sync"
)

type pawsJournalEntry struct {
	sequence uint64
	encoded  json.RawMessage
}
type pawsJournal struct {
	mu         sync.Mutex
	entries    []pawsJournalEntry
	bytes      int
	sequence   uint64
	maxEntries int
	maxBytes   int
}
type pawsJournalPage struct {
	Entries       []json.RawMessage `json:"entries"`
	NextCursor    uint64            `json:"nextCursor"`
	HighWatermark uint64            `json:"highWatermark"`
	DroppedBefore *uint64           `json:"droppedBefore"`
}

func newPawsJournal(maxEntries, maxBytes int) *pawsJournal {
	return &pawsJournal{maxEntries: maxEntries, maxBytes: maxBytes}
}
func (j *pawsJournal) append(encode func(sequence uint64) ([]byte, error)) error {
	j.mu.Lock()
	defer j.mu.Unlock()
	if j.sequence == math.MaxUint64 {
		return errors.New("journal sequence exhausted")
	}
	sequence := j.sequence + 1
	encoded, err := encode(sequence)
	if err != nil {
		return err
	}
	j.sequence = sequence
	if len(encoded) > j.maxBytes || j.maxEntries < 1 {
		j.entries = nil
		j.bytes = 0
		return nil
	}
	for len(j.entries) >= j.maxEntries || j.bytes > j.maxBytes-len(encoded) {
		j.bytes -= len(j.entries[0].encoded)
		j.entries[0] = pawsJournalEntry{}
		j.entries = j.entries[1:]
	}
	j.entries = append(j.entries, pawsJournalEntry{sequence, encoded})
	j.bytes += len(encoded)
	return nil
}
func (j *pawsJournal) page(after uint64, limit uint16) (pawsJournalPage, error) {
	j.mu.Lock()
	defer j.mu.Unlock()
	if limit < 1 || limit > 512 {
		return pawsJournalPage{}, errors.New("journal page limit must be 1..512")
	}
	if after > j.sequence {
		return pawsJournalPage{}, errors.New("cursor belongs to another session or future journal")
	}
	page := pawsJournalPage{Entries: make([]json.RawMessage, 0, int(limit)), NextCursor: after, HighWatermark: j.sequence}
	first := j.sequence
	if first != math.MaxUint64 {
		first++
	}
	if len(j.entries) > 0 {
		first = j.entries[0].sequence
	}
	if after < first && first-after > 1 {
		page.DroppedBefore = &first
	}
	for _, entry := range j.entries {
		if entry.sequence <= after {
			continue
		}
		page.Entries = append(page.Entries, entry.encoded)
		page.NextCursor = entry.sequence
		if len(page.Entries) == int(limit) {
			break
		}
	}
	if len(page.Entries) == 0 {
		page.NextCursor = j.sequence
	}
	return page, nil
}
func (j *pawsJournal) revision() uint64 {
	j.mu.Lock()
	defer j.mu.Unlock()
	return j.sequence
}
