dsb ishst
msr ttbr0_el1, {root}
dsb ish
tlbi vmalle1
dsb ish
isb
