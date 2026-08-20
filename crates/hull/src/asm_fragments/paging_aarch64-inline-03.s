msr mair_el1, {mair}
msr tcr_el1, {tcr}
msr ttbr0_el1, {root}
dsb ish
tlbi vmalle1
dsb ish
ic iallu
isb
