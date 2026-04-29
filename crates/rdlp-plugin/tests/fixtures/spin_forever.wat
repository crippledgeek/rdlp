(component
    (core module $m
        (func $spin (export "infinite")
            (loop $l (br $l)))
    )
    (core instance $i (instantiate $m))
    (func $f (export "infinite")
        (canon lift (core func $i "infinite")))
)
