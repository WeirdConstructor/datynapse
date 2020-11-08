!id2 = dn:timer:interval :ms => 2000;

dn:on (dn:timer:oneshot :ms => 1000) {
    std:displayln "timeout" _;
};

!cat = dn:process:start :line "cat" $[];
dn:on cat {
    std:displayln "cat:" @;
    ? std:str:trim[_1] == "QUIT" {
        dn:kill cat;
    };
};

dn:send cat "FOOBAR\n";
dn:send cat "FOOBAR\n";
dn:send cat "QUIT\n";

dn:on (dn:process:start
    :wsmp "sh" $["-c", "echo \"direct foo ba \\\"foo babab\\\"\""]) {

    std:displayln "in" @;
};

!cnt = 0;
dn:on (dn:process:start
    :lines "sh" $["-c", "while true; do date; sleep 1; done"]) {

    std:displayln "proc:" @;
    ? cnt > 10 {
        dn:kill _;
    };
    .cnt += 1;
};

dn:on id2 { std:displayln "timeout2" _; };
