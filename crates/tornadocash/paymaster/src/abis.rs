use alloy::sol;

sol!(
    struct PaymasterData {
        address adapter;
        bytes adapterData;
    }

    struct TornadoAdapterData {
        bytes proof;
        bytes32 root;
        bytes32 nullifierHash;
        address recipient;
        address relayer;
        uint256 fee;
        uint256 refund;
    }
);
