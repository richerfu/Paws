package main

import (
	"encoding/json"
	"math"
	"testing"
)

func TestJournalPagesDoNotSkipBacklogAndReportDroppedRecords(t *testing.T) {
	j := newPawsJournal(3, 1024)
	for i := 0; i < 5; i++ {
		if err := j.append(func(sequence uint64) ([]byte, error) { return json.Marshal(map[string]uint64{"sequence": sequence}) }); err != nil {
			t.Fatal(err)
		}
	}
	first, err := j.page(0, 1)
	if err != nil {
		t.Fatal(err)
	}
	if first.HighWatermark != 5 || first.NextCursor != 3 || first.DroppedBefore == nil || *first.DroppedBefore != 3 {
		t.Fatalf("first page: %#v", first)
	}
	second, _ := j.page(first.NextCursor, 1)
	if second.NextCursor != 4 || second.HighWatermark != 5 || second.DroppedBefore != nil {
		t.Fatalf("second: %#v", second)
	}
	third, _ := j.page(second.NextCursor, 1)
	if third.NextCursor != 5 || len(third.Entries) != 1 {
		t.Fatal(third)
	}
	empty, _ := j.page(5, 1)
	if empty.NextCursor != 5 || len(empty.Entries) != 0 {
		t.Fatal(empty)
	}
	if _, err := j.page(6, 1); err == nil {
		t.Fatal("future-session cursor accepted")
	}
}
func TestJournalByteBudgetAndSequenceOverflowAreExplicit(t *testing.T) {
	j := newPawsJournal(10, 6)
	if err := j.append(func(uint64) ([]byte, error) { return []byte(`"abcdef"`), nil }); err != nil {
		t.Fatal(err)
	}
	page, _ := j.page(0, 1)
	if len(page.Entries) != 0 || page.NextCursor != 1 || page.DroppedBefore == nil || *page.DroppedBefore != 2 {
		t.Fatal(page)
	}
	j.sequence = math.MaxUint64
	if err := j.append(func(uint64) ([]byte, error) { t.Fatal("encode after overflow"); return nil, nil }); err == nil {
		t.Fatal("overflow accepted")
	}
}
