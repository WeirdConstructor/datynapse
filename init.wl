!id2 = dn:timer:interval :ms => 2000;

dn:on (dn:timer:oneshot :ms => 1000) {
    std:displayln "timeout" _;
};

dn:on id2 { std:displayln "timeout2" _; };
