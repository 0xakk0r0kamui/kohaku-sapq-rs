use alloy::sol;

sol!(
    #[sol(rpc)]
    contract TornadoPaymaster {
        function quoteWeiInToken(address feeToken, uint256 weiAmount) external view returns (uint256 tokenAmount);
    }

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
