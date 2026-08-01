// Month-grid math for the in-app date picker. Dates are "YYYY-MM-DD" strings
// (the issue storage format); month numbers are 1-12. Weeks start Monday.

export interface DayCell {
  date: string;
  day: number;
  inMonth: boolean;
}

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

const pad = (n: number) => String(n).padStart(2, "0");
const dstr = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

// Always 6 rows of 7 — a constant-height grid, so paging months never makes
// the popover jump under the pointer.
export function monthGrid(year: number, month: number): DayCell[][] {
  const first = new Date(year, month - 1, 1);
  const lead = (first.getDay() + 6) % 7; // Monday-first offset
  const weeks: DayCell[][] = [];
  for (let i = 0; i < 42; i++) {
    const d = new Date(year, month - 1, 1 - lead + i);
    if (i % 7 === 0) weeks.push([]);
    weeks[weeks.length - 1].push({
      date: dstr(d),
      day: d.getDate(),
      inMonth: d.getMonth() === month - 1,
    });
  }
  return weeks;
}

export function monthLabel(year: number, month: number): string {
  return `${MONTHS[month - 1]} ${year}`;
}

export function shiftMonth(year: number, month: number, delta: number): [number, number] {
  const n = year * 12 + (month - 1) + delta;
  return [Math.floor(n / 12), (((n % 12) + 12) % 12) + 1];
}
