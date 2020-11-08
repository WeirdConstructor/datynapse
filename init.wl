# TODO: dn:process:filter   - for each received message the process is fired up
#                             with the given input.
#                             One response message is generated.
#
# TODO: dn:process:generate - fire up a process directly without dn:send with EOF on stdin.
#
# TODO: Add $p(:stream, 4094) type instead of :lines, which allocates a buffer
# and reads the data that it gets in max chunks of the given buffer size.
# EOF generates an empty message.
# TODO: Add :collect which collects all output until EOF into a single message.
#
# TODO: dn:file:read    - with :lines line based, with :stream binary based
#
# TODO: dn:file:new_in_dir - Reads in new files in a directory :lines, :stream or :collect based.
#                            The messages all contain the file name.
# TODO: dn:file:to_dir - Each received message is written to a file in the output
#                        directory. Filename and contents are in the message.
#                        Files are written atomically with "~".
# TODO: dn:file:append - Each received message is appended to a given file.
#                        It's easy to build a log with this!
#
# TODO: Write tests! Split up the stuff below and write wlambda tests!

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
