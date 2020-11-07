!id = dn:timer:oneshot :ms => 1000;
!id2 = dn:timer:oneshot :ms => 2000;

dn:on id {
    std:displayln "timeout";
};

dn:on id2 {
    std:displayln "timeout2";
};
